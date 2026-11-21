//! Writer of `.cdr` drawings of version 15: a ZIP package (R24, R25, R64) whose member
//! `content/riffData.cdr` is the RIFF file. Each record is laid out as the reader in this crate
//! reads it, by the `PROVENANCE.md` rows the code cites, so a written file reads back as the
//! drawing it was written from. Records the reader does not read are not written.
//!
//! Writing is bounded by [`Limits`]: a drawing the reader would not read whole within the same
//! limits is refused. It never recurses and does not panic.

use std::cell::Cell;
use std::collections::HashMap;
use std::io::Write as _;
use std::rc::Rc;

use self::WriteError as Error;
use crate::Limits;
use crate::model::{
    Blend, Cap, Color, Drawing, Flags, Geometry, Guideline, Join, LayerKind, Matrix, Object, Outline, Page, Path, Pt, Segment, Shape, SubPath,
};

/// Why a drawing can't be written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WriteError {
    /// The reader would not read the file whole within the [`Limits`] it was written for.
    Limit(&'static str),
    /// What the file can't hold, or what the reader would read back changed.
    Unwritable(&'static str),
}

impl std::fmt::Display for WriteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WriteError::Limit(what) => write!(f, "past the {what} limit"),
            WriteError::Unwritable(what) => write!(f, "{what} can't be written"),
        }
    }
}

impl std::error::Error for WriteError {}

/// The version written, in hundreds, as its `vrsn` chunk names it (R2).
pub const WRITTEN_VERSION: u16 = 1500;
/// The form type of version 15: `CDR` and the letter `F` (R1).
const FORM: [u8; 4] = *b"CDRF";
/// The package member that holds the RIFF file (R25).
const MEMBER: &[u8] = b"content/riffData.cdr";
/// The package's first member and what it holds (R64).
const MIMETYPE_NAME: &[u8] = b"mimetype";
const MIMETYPE: &[u8] = b"application/x-vnd.corel.draw.document+zip";
/// Coordinate units (1/254000 inch) in a point (R5).
const UNITS_PER_PT: f64 = 254_000.0 / 72.0;
/// The tag of the record layout from version 13 (R11, R12, R47).
const TAG_1300: u32 = 1300;

/// Object types (R6, R23).
const RECTANGLE: u32 = 1;
const ELLIPSE: u32 = 2;
const CURVE: u32 = 3;
const GUIDELINE: u32 = 0x16;
/// Argument types (R6, R20, R21, R23, R46, R47, R52, R70).
const ARG_OUTLINE: u32 = 10;
const ARG_FILL: u32 = 20;
const ARG_COORDS: u32 = 30;
const ARG_NAME: u32 = 1000;
const ARG_TRANSPARENCY: u32 = 8000;
const ARG_CONTAINER: u32 = 8005;
const ARG_WINDING: u32 = 14001;
const ARG_GUIDE: u32 = 16000;
const ARG_PAGE_SIZE: u32 = 19130;
const ARG_LAYER_ID: u32 = 40050;

/// The most clipped groups the reader reads one inside another (`build.rs`).
const MAX_CLIP_DEPTH: usize = 64;
/// Chunks the reader counts for one object list of a clipped group, as [`Writer::shape`] lays it
/// out: the list itself, then its searches (`build.rs`, `find_own`) for its `loda` (`flgs`, `lgob`,
/// `loda`), its `flgs` (`flgs`) and its `trfd` (`flgs`, `lgob`, `loda`, `trfl`, `trfd`).
const OBJECT_WALK: usize = 1 + 3 + 1 + 5;
/// Dash and gap lengths an outline record has room for: the arrowhead ids follow 20 bytes after
/// the first dash (R12).
const MAX_DASHES: usize = 10;
/// The most UTF-16 units of a name the reader keeps whole (`records.rs`).
const MAX_NAME: usize = 256;
/// Where a guideline's second point lies along it, in points: the writer's own value.
const GUIDE_REACH: f64 = 100.0;
/// How far, in degrees, the line through a guideline's two points, once rounded, may turn from
/// its stored angle: the writer's own value.
const GUIDE_TURN: f64 = 1.0;
/// How many units along the longer axis a guideline's second point is looked for in when no
/// farther point lies along its angle: past 82 units, a rounded point turns less than
/// `GUIDE_TURN`.
const GUIDE_STEPS: i32 = 128;

/// The `.cdr` file of `drawing`, version 15, within `limits`.
///
/// Pages, the master page's layers and the drawing's guidelines, layers with their kinds, names
/// and flags, groups, clipping containers with the objects they clip, curves, rectangles with
/// their corners, ellipses, uniform fills, outlines, the nonzero fill rule and uniform
/// transparency with its blend mode are written. Every other page names the master page's guides,
/// desktop and grid layers by their layer ids, and not its drawing layers. The drawing's `version`
/// and `warnings` are not written, and the written file reads back with these changes:
///
/// - Groups that hold no shape or container are left out, and a container whose objects hold none
///   is written as a plain shape. A subpath without segments is left out. A drawing without pages
///   gets one empty page.
/// - A closed subpath whose last segment ends away from its start gets a closing line to its
///   start, as the file closes subpaths with an explicit point.
/// - Guidelines on the master page's layers read back as the drawing's guidelines. An empty layer
///   of the master page, and an empty layer of a page other than a drawing layer, read back left
///   out, as the reader leaves out such layers in any file.
/// - A guideline on a hidden layer reads back hidden, as the reader marks every guideline of a
///   hidden layer. A guideline angle past the field's range is written as the same angle within
///   one turn.
/// - Dash lengths are rounded to whole outline widths and spot color tints to whole percent, as
///   the file stores them. A spot color without a look is written with a black look.
///
/// Hidden groups and objects carry the same hide bit as hidden layers (R19). The specification's
/// notes date that bit on groups and objects to a later program release than version 15 files, so
/// a program of that version may show them.
///
/// Fails with [`WriteError::Limit`] when the reader would not read the file whole within `limits`
/// or groups nest deeper than its nesting depth, groups that hold nothing included, and with
/// [`WriteError::Unwritable`] for what the file can't hold or the reader would read back otherwise
/// changed: a coordinate past 32 bits of 1/254000 inch; a size, matrix or angle that is
/// not finite; a page width or height that rounds to less than one coordinate unit; a negative
/// outline width, or a positive one that rounds to 0; a corner radius below 0 or more than a billionth past half the rectangle's
/// shorter side (a radius within that is written as half); a color value, tint or opacity outside
/// 0 to 1; more than 10 dash lengths, one that is negative, not finite or rounds past 65,535, or a
/// pattern whose lengths all round to 0; a name longer than 256 UTF-16 units or holding a NUL; a
/// spot color without a name, at a tint that rounds to 0 percent or at a tint too low for its look
/// to read back within one step of 1/255; objects on the master page's grid layer; a guideline at
/// the edge of the coordinate range with no room for a second point on it whose rounded
/// coordinates lie within 1 degree of its angle.
pub fn write(drawing: &Drawing, limits: Limits) -> Result<Vec<u8>, Error> {
    // Every package holds two members (R64).
    if limits.members < 2 {
        return Err(Error::Limit("package member count"));
    }
    let mut w = Writer::new(limits);
    // The chunks the `doc` list always holds count from the start: `vrsn`, `doc `, `filt` and
    // `otlt`, and `mcfg` with a page size.
    w.counted.set(4 + usize::from(drawing.page_size.is_some()));
    let mut pages = Out::new(0, limits, w.total.clone(), w.counted.clone());
    w.master_page(&mut pages, drawing)?;
    if drawing.pages.is_empty() {
        // A drawing has at least one page, with one layer.
        pages.open(b"page")?;
        pages.chunk(b"flgs", &PAGE_FLAGS)?;
        pages.open(b"gobj")?;
        w.master_refs(&mut pages, false)?;
        let id = w.layer_id()?;
        w.layer(&mut pages, &LayerParts { name: "Layer 1", kind: LayerKind::Drawing, flags: Flags::default(), master: false, id }, &[], [&[], &[]])?;
        w.master_refs(&mut pages, true)?;
        pages.close()?;
        pages.close()?;
    }
    for page in &drawing.pages {
        w.page(&mut pages, page)?;
    }
    let clips = w.clipped_groups()?;
    // The bytes of each layer id after its count come from hashes of what is written, so
    // different drawings get different ids and the same drawing the same ones (R70).
    // A CRC-32 and a 64-bit FNV-1a hash of the same bytes.
    let config = drawing.page_size.map(mcfg).transpose()?;
    let (mut crc, mut fnv) = (crc32fast::Hasher::new(), 0xcbf2_9ce4_8422_2325_u64);
    let mut hash = |part: &[u8]| {
        crc.update(part);
        for b in part {
            fnv = (fnv ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3);
        }
    };
    for part in [&pages.buf[..], clips.as_ref().map_or(&[][..], |c| &c.buf), config.as_deref().unwrap_or_default()] {
        hash(part);
    }
    // Each record as its chunk holds it: the id, then the body.
    for (id, body) in w.fills.bodies.iter().chain(&w.outlines.bodies) {
        hash(&id.to_le_bytes());
        hash(body);
    }
    let mut salt = [0u8; 12];
    for (slot, b) in salt.iter_mut().zip(crc.finalize().to_le_bytes().into_iter().chain(fnv.to_le_bytes())) {
        *slot = b;
    }
    for &at in &w.id_at {
        if let Some(slot) = pages.buf.get_mut(at + 4..at + 16) {
            slot.copy_from_slice(&salt);
        }
    }
    // The `doc` list's count starts with the RIFF header and the pages, which follow it.
    // Its chunk count starts with the chunks of the pages and the clipped groups.
    let streams = pages.chunks.saturating_add(clips.as_ref().map_or(0, |c| c.chunks));
    let mut doc = Out::new(0, limits, Rc::new(Cell::new(pages.buf.len().saturating_add(12))), Rc::new(Cell::new(streams)));
    // The version chunk comes first (R2).
    doc.chunk(b"vrsn", &WRITTEN_VERSION.to_le_bytes())?;
    doc.open(b"doc ")?;
    if let Some(config) = &config {
        doc.chunk(b"mcfg", config)?;
    }
    // Each fill record in a `LIST filc` of its own (R72).
    doc.open(b"filt")?;
    for (id, body) in &w.fills.bodies {
        doc.open(b"filc")?;
        doc.chunk_of(b"fild", &[&id.to_le_bytes(), body])?;
        doc.close()?;
    }
    doc.close()?;
    doc.open(b"otlt")?;
    for (id, body) in &w.outlines.bodies {
        doc.chunk_of(b"outl", &[&id.to_le_bytes(), body])?;
    }
    doc.close()?;
    // The clipped groups end the `doc` list; their bytes follow its stream in the file.
    let clipped = clips.as_ref().map_or(&[][..], |c| &c.buf);
    if clips.is_some() {
        doc.tail(b"clpt", clipped.len())?;
    }
    doc.close()?;
    // The reader takes the RIFF file within the input size too, once it is out of the package.
    let len = 12usize.saturating_add(doc.buf.len()).saturating_add(clipped.len()).saturating_add(pages.buf.len());
    fits_stream(&limits, len)?;
    let mut head = b"RIFF".to_vec();
    head.extend(len32(len - 8)?.to_le_bytes());
    head.extend(FORM);
    let file = package(&[&head, &doc.buf, clipped, &pages.buf])?;
    if file.len() > limits.input {
        return Err(Error::Limit("input size"));
    }
    Ok(file)
}

/// Checks that `n` bytes of the RIFF file fit what a read takes: it takes the file as input, as an
/// inflated package member and within the inflated bytes of the whole file. The error names the
/// smallest of those limits.
fn fits_stream(limits: &Limits, n: usize) -> Result<(), Error> {
    let (max, what) = [(limits.input, "input size"), (limits.member, "member size"), (limits.inflated, "inflated size")]
        .into_iter()
        .min_by_key(|(max, _)| *max)
        .unwrap_or((0, "input size"));
    if n > max { Err(Error::Limit(what)) } else { Ok(()) }
}

/// A u32 length field, or the size limit.
fn len32(n: usize) -> Result<u32, Error> {
    u32::try_from(n).map_err(|_| Error::Limit("output size"))
}

/// A chunk stream being written: chunks and lists, with each list's size filled in when it closes
/// (R4).
struct Out {
    buf: Vec<u8>,
    /// Where each open list's size field lies.
    open: Vec<usize>,
    /// Lists around this stream's own in the file.
    base: usize,
    /// Chunks written, lists included.
    chunks: usize,
    limits: Limits,
    /// Bytes of the RIFF file written so far, by every stream that shares the count.
    total: Rc<Cell<usize>>,
    /// Chunks of the RIFF file counted so far, lists included, by every stream that shares the
    /// count.
    counted: Rc<Cell<usize>>,
    /// Bytes that follow the stream in the file and belong to its open lists (see `tail`).
    after: usize,
}

impl Out {
    fn new(base: usize, limits: Limits, total: Rc<Cell<usize>>, counted: Rc<Cell<usize>>) -> Out {
        Out { buf: Vec::new(), open: Vec::new(), base, chunks: 0, limits, total, counted, after: 0 }
    }

    fn count(&mut self) -> Result<(), Error> {
        self.chunks = self.chunks.saturating_add(1);
        self.counted.set(self.counted.get().saturating_add(1));
        self.room_for_chunks(0)
    }

    fn room_for_chunks(&self, more: usize) -> Result<(), Error> {
        if self.counted.get().saturating_add(more) > self.limits.chunks { Err(Error::Limit("chunk count")) } else { Ok(()) }
    }

    /// Checks that `lists` more lists nested in the open ones and `chunks` more chunks, lists
    /// included, fit what a read takes, before anything is written.
    fn fits(&self, lists: usize, chunks: usize) -> Result<(), Error> {
        if self.base + self.open.len() + lists > self.limits.depth {
            return Err(Error::Limit("nesting depth"));
        }
        self.room_for_chunks(chunks)
    }

    /// Checks that `n` more bytes keep the file within what a read takes.
    fn room(&self, n: usize) -> Result<(), Error> {
        fits_stream(&self.limits, self.total.get().saturating_add(n))
    }

    fn put(&mut self, bytes: &[u8]) {
        self.buf.extend(bytes);
        self.total.set(self.total.get().saturating_add(bytes.len()));
    }

    fn chunk(&mut self, id: &[u8; 4], body: &[u8]) -> Result<(), Error> {
        self.chunk_of(id, &[body])
    }

    /// A chunk whose body is `parts`, one after another.
    fn chunk_of(&mut self, id: &[u8; 4], parts: &[&[u8]]) -> Result<(), Error> {
        let len = parts.iter().fold(0usize, |n, p| n.saturating_add(p.len()));
        self.count()?;
        self.room(len.saturating_add(8 + len % 2))?;
        self.put(id);
        self.put(&len32(len)?.to_le_bytes());
        for p in parts {
            self.put(p);
        }
        if len % 2 == 1 {
            self.put(&[0]);
        }
        Ok(())
    }

    /// A `loda` chunk of a record of object type `kind` (R6): length, argument count, where the
    /// offsets and the types lie, the type; the arguments; the offsets with one more for the end;
    /// the types in reverse. The arguments are copied straight into the stream.
    fn loda(&mut self, kind: u32, args: &[(u32, Vec<u8>)]) -> Result<(), Error> {
        const HEAD: usize = 20;
        let body = args.iter().fold(0usize, |n, (_, a)| n.saturating_add(a.len()));
        let offsets_at = HEAD.saturating_add(body);
        let types_at = offsets_at.saturating_add((args.len() + 1).saturating_mul(4));
        let len = types_at.saturating_add(args.len().saturating_mul(4));
        self.count()?;
        self.room(len.saturating_add(8 + len % 2))?;
        self.put(b"loda");
        self.put(&len32(len)?.to_le_bytes());
        for v in [len, args.len(), offsets_at, types_at] {
            self.put(&len32(v)?.to_le_bytes());
        }
        self.put(&kind.to_le_bytes());
        for (_, a) in args {
            self.put(a);
        }
        let mut at = HEAD;
        for (_, a) in args {
            self.put(&len32(at)?.to_le_bytes());
            at += a.len();
        }
        self.put(&len32(at)?.to_le_bytes());
        for (t, _) in args.iter().rev() {
            self.put(&t.to_le_bytes());
        }
        if len % 2 == 1 {
            self.put(&[0]);
        }
        Ok(())
    }

    /// Opens a list of `form`. The reader reads the chunks of a list nested at most `depth` deep.
    fn open(&mut self, form: &[u8; 4]) -> Result<(), Error> {
        self.count()?;
        if self.base + self.open.len() + 1 > self.limits.depth {
            return Err(Error::Limit("nesting depth"));
        }
        self.room(12)?;
        self.put(b"LIST");
        self.open.push(self.buf.len());
        self.put(&[0; 4]);
        self.put(form);
        Ok(())
    }

    fn close(&mut self) -> Result<(), Error> {
        let at = self.open.pop().ok_or(Error::Limit("output size"))?;
        let len = (self.buf.len() + self.after) - at - 4;
        let field = len32(len)?.to_le_bytes();
        if let Some(slot) = self.buf.get_mut(at..at + 4) {
            slot.copy_from_slice(&field);
        }
        if len % 2 == 1 {
            // A pad byte would go after the bytes kept outside the stream.
            if self.after > 0 {
                return Err(Error::Limit("output size"));
            }
            self.put(&[0]);
        }
        Ok(())
    }

    /// Opens a list of `form` whose chunks, `len` bytes already laid out, follow the stream in
    /// the file instead of being copied into it. Nothing more is written to the stream; only the
    /// lists open around it close.
    fn tail(&mut self, form: &[u8; 4], len: usize) -> Result<(), Error> {
        self.open(form)?;
        self.room(len)?;
        self.after = len;
        self.total.set(self.total.get().saturating_add(len));
        self.close()
    }
}

/// Fill or outline records by their body, numbered as they are first used.
#[derive(Default)]
struct Records {
    ids: HashMap<Rc<[u8]>, u32>,
    /// Each record's id and body, in the order they were numbered; the chunk holds the id, then
    /// the body.
    bodies: Vec<(u32, Rc<[u8]>)>,
}

/// What the read of the written file will count, against the limits.
struct Writer<'d> {
    limits: Limits,
    /// Bytes of the RIFF file written so far: the pages, the clipped groups and the fill and
    /// outline tables, held together to what a read takes.
    total: Rc<Cell<usize>>,
    /// Chunks of the RIFF file counted so far, held to the chunk limit: the `doc` list's own, the
    /// pages', the clipped groups' and the records'.
    counted: Rc<Cell<usize>>,
    fills: Records,
    outlines: Records,
    /// The next fill or outline id.
    next_id: u32,
    /// The objects each container clips, by the id of its `LIST clpt` group (R46), with how many
    /// clipped groups the reader has open when it reads them.
    clips: Vec<(u32, &'d [Object], usize)>,
    /// The clipped groups open around the objects being written.
    clip_depth: usize,
    objects: usize,
    coordinates: usize,
    dashes: usize,
    /// Chunks the reader steps through inside clipped groups: each group's `spnd` and flags, each
    /// object or group list, each nested group's flags, and the chunks it looks at to find each
    /// object's `loda`, `flgs` and `trfd`.
    clip_walked: usize,
    /// Layer ids given out so far, and where each layer's id lies in the pages' stream.
    layer_ids: u32,
    id_at: Vec<usize>,
    /// The master page's guides, desktop and grid layers, which the other pages name (R70).
    master_refs: Vec<(&'d str, LayerKind, Flags, [u8; 16])>,
}

/// What a layer list carries besides its objects and guidelines.
struct LayerParts<'a> {
    name: &'a str,
    kind: LayerKind,
    flags: Flags,
    /// On the master page, or an entry that names one of its layers.
    master: bool,
    /// The layer id (R70).
    id: [u8; 16],
}

/// A layer of the master page as it is written.
struct MasterLayer<'a> {
    name: &'a str,
    kind: LayerKind,
    flags: Flags,
    objects: &'a [Object],
    guides: [&'a [Guideline]; 2],
}

/// A layer id before its last 12 bytes are set for the drawing: the first four count the layers
/// (R70).
const LAYER_ID: [u8; 16] = [0; 16];

/// The flags of a page other than the master page (R19).
const PAGE_FLAGS: [u8; 4] = [0, 0, 0, 0x90];

impl<'d> Writer<'d> {
    fn new(limits: Limits) -> Self {
        Writer {
            limits,
            total: Rc::new(Cell::new(0)),
            counted: Rc::new(Cell::new(0)),
            fills: Records::default(),
            outlines: Records::default(),
            next_id: 1,
            clips: Vec::new(),
            clip_depth: 0,
            objects: 0,
            coordinates: 0,
            dashes: 0,
            clip_walked: 0,
            layer_ids: 0,
            id_at: Vec::new(),
            master_refs: Vec::new(),
        }
    }

    /// A new layer id (R70), different from every other id in the file.
    fn layer_id(&mut self) -> Result<[u8; 16], Error> {
        self.layer_ids = self.layer_ids.checked_add(1).ok_or(Error::Limit("output size"))?;
        let mut id = LAYER_ID;
        for (slot, b) in id.iter_mut().zip(self.layer_ids.to_le_bytes()) {
            *slot = b;
        }
        Ok(id)
    }

    /// The entries that name the master page's guides, desktop and grid layers on another page
    /// (R70): the grid layers' when `grids`, the others' otherwise.
    fn master_refs(&mut self, out: &mut Out, grids: bool) -> Result<(), Error> {
        let refs: Vec<_> = self.master_refs.iter().filter(|r| (r.1 == LayerKind::Grid) == grids).copied().collect();
        for (name, kind, flags, id) in refs {
            self.layer(out, &LayerParts { name, kind, flags, master: true, id }, &[], [&[], &[]])?;
        }
        Ok(())
    }

    /// A `loda` record, within the arguments a read takes from one.
    fn loda(&self, out: &mut Out, kind: u32, args: &[(u32, Vec<u8>)]) -> Result<(), Error> {
        if args.len() > self.limits.arguments {
            return Err(Error::Limit("argument count"));
        }
        out.loda(kind, args)
    }

    /// `n` more chunks the reader steps through, counted against the clipped chunks while inside
    /// a clipped group.
    fn walk(&mut self, n: usize) -> Result<(), Error> {
        if self.clip_depth == 0 {
            return Ok(());
        }
        self.clip_walked = self.clip_walked.saturating_add(n);
        if self.clip_walked > self.limits.clipped_chunks { Err(Error::Limit("clipped chunk count")) } else { Ok(()) }
    }

    fn take_object(&mut self) -> Result<(), Error> {
        self.objects += 1;
        if self.objects > self.limits.objects { Err(Error::Limit("object count")) } else { Ok(()) }
    }

    /// The master page (R19, R70).
    fn master_page(&mut self, out: &mut Out, drawing: &'d Drawing) -> Result<(), Error> {
        out.open(b"page")?;
        // Byte 2 marks the master page (R19).
        out.chunk(b"flgs", &[0, 0, 1, 0x90])?;
        out.open(b"gobj")?;
        // The reader leaves out the master page's grid layers.
        for l in drawing.master.iter().filter(|l| l.kind == LayerKind::Grid) {
            if draws(&l.objects, &self.limits)? {
                return Err(Error::Unwritable("objects on the master page's grid layer"));
            }
        }
        // The drawing's master layers, with a guides, a desktop and a grid layer when they have
        // none. The drawing's guidelines go in the first guides layer that is shown and unlocked,
        // and in a guides layer of their own when there is none.
        let made = |name, kind, flags, guides| MasterLayer { name, kind, flags, objects: &[], guides: [guides, &[]] };
        let first_guides = drawing.master.iter().position(|l| l.kind == LayerKind::Guides && l.flags == Flags::default());
        let has_guides = drawing.master.iter().any(|l| l.kind == LayerKind::Guides);
        let own_guides = first_guides.is_none() && (!has_guides || !drawing.guides.is_empty());
        let guides_layer = own_guides.then(|| made("Guides", LayerKind::Guides, Flags::default(), &drawing.guides[..]));
        let layers = drawing.master.iter().enumerate().map(|(i, l)| MasterLayer {
            name: &l.name,
            kind: l.kind,
            flags: l.flags,
            objects: &l.objects,
            guides: [&l.guides, if Some(i) == first_guides { &drawing.guides } else { &[] }],
        });
        let has_desktop = drawing.master.iter().any(|l| l.kind == LayerKind::Desktop);
        let desktop = (!has_desktop).then(|| made("Desktop", LayerKind::Desktop, Flags::default(), &[]));
        // A grid layer, hidden and locked as sample files hold it, when they have none; the reader
        // leaves out the master page's grid layers (R70).
        let has_grid = drawing.master.iter().any(|l| l.kind == LayerKind::Grid);
        let grid = (!has_grid).then(|| made("Grid", LayerKind::Grid, Flags { hidden: true, locked: true }, &[]));
        for l in guides_layer.into_iter().chain(layers).chain(desktop).chain(grid) {
            let id = self.layer_id()?;
            if l.kind != LayerKind::Drawing {
                self.master_refs.push((l.name, l.kind, l.flags, id));
            }
            self.layer(out, &LayerParts { name: l.name, kind: l.kind, flags: l.flags, master: true, id }, l.objects, l.guides)?;
        }
        out.close()?;
        out.close()
    }

    /// A page (R19, R21, R22): its flags, its size in its own `loda`, then its layers in a `gobj`
    /// list.
    fn page(&mut self, out: &mut Out, page: &'d Page) -> Result<(), Error> {
        out.open(b"page")?;
        out.chunk(b"flgs", &PAGE_FLAGS)?;
        if let Some(size) = page.size {
            let size = page_size(size)?;
            out.open(b"lgob")?;
            self.loda(out, 0, &[(ARG_PAGE_SIZE, size)])?;
            out.close()?;
        }
        out.open(b"gobj")?;
        // The entries for the master page's layers go around the page's own, as sample files
        // place them (R70).
        self.master_refs(out, false)?;
        for l in &page.layers {
            let id = self.layer_id()?;
            self.layer(out, &LayerParts { name: &l.name, kind: l.kind, flags: l.flags, master: false, id }, &l.objects, [&l.guides, &[]])?;
        }
        self.master_refs(out, true)?;
        out.close()?;
        out.close()
    }

    /// A layer (R19, R9): flags, its `loda` with its kind as the object type and its name, then
    /// its objects and guidelines.
    fn layer(&mut self, out: &mut Out, layer: &LayerParts, objects: &'d [Object], guides: [&[Guideline]; 2]) -> Result<(), Error> {
        // The kind's low bits; bit 0x10, which also marks a grid, is the lock bit here, and the
        // object type names the kind.
        let (kind, byte0) = match layer.kind {
            LayerKind::Drawing => (0, 0),
            LayerKind::Grid => (11, 0x0a),
            LayerKind::Guides => (12, 0x0a),
            LayerKind::Desktop => (17, 0x08),
        };
        out.open(b"layr")?;
        // Byte 2 is set on the master page's layers and on the entries that name them.
        out.chunk(b"flgs", &flags(layer.flags, byte0, u8::from(layer.master), 0x98))?;
        out.open(b"lgob")?;
        // The id is the first argument: after the chunk's header and the record's 20 bytes.
        self.id_at.push(out.buf.len() + 8 + 20);
        self.loda(out, kind, &[(ARG_LAYER_ID, layer.id.to_vec()), (ARG_NAME, name(layer.name)?)])?;
        out.close()?;
        self.objects(out, objects)?;
        for g in guides.into_iter().flatten() {
            self.guideline(out, g)?;
        }
        out.close()
    }

    /// A guideline object (R23).
    fn guideline(&mut self, out: &mut Out, g: &Guideline) -> Result<(), Error> {
        self.take_object()?;
        // The points lie along the angle as stored.
        let stored = angle(g.angle)?;
        let (s, c) = (f64::from(i32::from_le_bytes(stored)) / 1_000_000.0).to_radians().sin_cos();
        let along = |k: f64| -> Result<Vec<u8>, Error> { Ok([coord(g.at.x + k * c)?, coord(g.at.y + k * s)?].concat()) };
        let first = along(0.0)?;
        // Whether a rounded second point lies apart from the first along the stored angle, on
        // either side, within `GUIDE_TURN`.
        let units = |p: &[u8], i: usize| p.get(4 * i..4 * i + 4).and_then(|b| b.try_into().ok()).map_or(0.0, |b| f64::from(i32::from_le_bytes(b)));
        let along_angle = |p: &Vec<u8>| {
            let (dx, dy) = (units(p, 0) - units(&first, 0), units(p, 1) - units(&first, 1));
            // With room for the floating-point error of a turn of exactly `GUIDE_TURN`.
            (dx != 0.0 || dy != 0.0) && (dx * s - dy * c).abs().atan2((dx * c + dy * s).abs()) <= GUIDE_TURN.to_radians() + 1e-12
        };
        // A second point on the line: the point at the full reach, on the positive side first.
        // When neither side has one in the file's range, the farthest point on each side whose
        // coordinates round within the range, and of the two the one farther away. Rounding moves
        // monotonically along the line, so the points that round within the range form one span
        // around the first point; halving the step finds its end to the last representable
        // value. Short of a farthest point along the angle, which only happens within
        // `GUIDE_STEPS` units of the range's edge, the farthest rounded point of the line within
        // that distance that lies along the angle, on either side.
        let full = [GUIDE_REACH, -GUIDE_REACH].into_iter().find_map(|k| along(k).ok().filter(along_angle));
        let stepped = || {
            // Each side's run of rounded points, walked outward from the start with `along`
            // itself: a step of one unit's length at a time, and a halving search for where the
            // rounded point changes, so every rounded point the line passes through is reached.
            let reach = f64::from(GUIDE_STEPS) / (UNITS_PER_PT * c.abs().max(s.abs()));
            let unit = 1.0 / UNITS_PER_PT;
            let mut best: Option<(f64, Vec<u8>)> = None;
            for side in [1.0, -1.0] {
                let (mut k, mut current) = (0.0, first.clone());
                loop {
                    let next = |k: f64| side * (k.abs() + unit).min(reach);
                    let mut far = next(k);
                    while along(far).ok().as_ref() == Some(&current) && far.abs() < reach {
                        k = far;
                        far = next(k);
                    }
                    if along(far).ok().as_ref() == Some(&current) {
                        break;
                    }
                    let (mut same, mut changed) = (k, far);
                    loop {
                        let mid = same + (changed - same) / 2.0;
                        if mid == same || mid == changed {
                            break;
                        }
                        if along(mid).ok().as_ref() == Some(&current) { same = mid } else { changed = mid }
                    }
                    let Ok(p) = along(changed) else { break };
                    if along_angle(&p) && best.as_ref().is_none_or(|b| changed.abs() > b.0.abs()) {
                        best = Some((changed, p.clone()));
                    }
                    (k, current) = (changed, p);
                }
            }
            best.map(|(_, p)| p)
        };
        let farthest = |end: f64| -> Option<(f64, Vec<u8>)> {
            let (mut ok, mut bad) = (0.0, end);
            loop {
                let mid = ok + (bad - ok) / 2.0;
                if mid == ok || mid == bad {
                    break;
                }
                if along(mid).is_ok() { ok = mid } else { bad = mid }
            }
            along(ok).ok().map(|p| (ok, p))
        };
        let second = full
            .or_else(|| {
                [farthest(GUIDE_REACH), farthest(-GUIDE_REACH)]
                    .into_iter()
                    .flatten()
                    .filter(|(_, p)| along_angle(p))
                    .reduce(|a, b| if b.0.abs() > a.0.abs() { b } else { a })
                    .map(|(_, p)| p)
            })
            .or_else(stepped)
            .ok_or(Error::Unwritable("guidelines with no room for a second point along their angle"))?;
        let mut arg = vec![0; 12];
        arg.extend(stored);
        arg.extend(&first);
        arg.extend(second);
        arg.extend(first);
        out.open(b"obj ")?;
        out.chunk(b"flgs", &flags(Flags { hidden: g.hidden, locked: false }, 0, 0, 0x08))?;
        out.open(b"lgob")?;
        self.loda(out, GUIDELINE, &[(ARG_GUIDE, arg)])?;
        out.close()?;
        out.close()
    }

    /// `objects`, front to back, groups nested, without recursion.
    fn objects(&mut self, out: &mut Out, objects: &'d [Object]) -> Result<(), Error> {
        // The objects left at each level, the layer's and then each group's, with the group's
        // flags. The reader leaves out a group that holds no shape or container, so a group's list
        // is written only when the first one is reached: the first `written` levels have theirs.
        let mut levels = vec![(objects.iter(), Flags::default())];
        let mut written = 1;
        loop {
            let Some((level, _)) = levels.last_mut() else { return Ok(()) };
            let Some(object) = level.next() else {
                levels.pop();
                if levels.is_empty() {
                    return Ok(());
                }
                if written > levels.len() {
                    written = levels.len();
                    // The reader counts a group once its objects are read.
                    self.take_object()?;
                    out.close()?;
                }
                continue;
            };
            if let Object::Group { objects, flags } = object {
                // Groups nested past the depth are refused even when they hold nothing, so the
                // walk holds no more levels than that. This group is `levels.len()` deep.
                if levels.len() > self.limits.depth {
                    return Err(Error::Limit("nesting depth"));
                }
                if !objects.is_empty() {
                    levels.push((objects.iter(), *flags));
                }
                continue;
            }
            for (_, f) in levels.get(written..).unwrap_or_default() {
                self.walk(2)?;
                out.open(b"grp ")?;
                out.chunk(b"flgs", &flags(*f, 0, 0, 0x10))?;
            }
            written = levels.len();
            match object {
                Object::Shape(s) => {
                    self.walk(OBJECT_WALK)?;
                    self.shape(out, s, None)?;
                }
                Object::Group { .. } => {}
                Object::Clip { container, objects } => {
                    self.walk(OBJECT_WALK)?;
                    // The reader reads a container whose objects hold no shape as a plain shape.
                    if !draws(objects, &self.limits)? {
                        self.shape(out, container, None)?;
                        continue;
                    }
                    let depth = self.clip_depth + 1;
                    if depth > MAX_CLIP_DEPTH {
                        return Err(Error::Limit("clip nesting"));
                    }
                    // Ids 1, 2, 3 ..., the same in the container and its group's `spnd` (R46, R73).
                    let id = len32(self.clips.len() + 1)?;
                    if self.clips.is_empty() {
                        // The `doc` list's `LIST clpt`, which holds the clipped groups.
                        self.counted.set(self.counted.get().saturating_add(1));
                    }
                    self.clips.push((id, objects, depth));
                    self.shape(out, container, Some(id))?;
                }
            }
        }
    }

    /// The `LIST clpt` body: one group for each container, holding the objects it clips (R46);
    /// `None` without containers. The groups' own objects may name more groups.
    fn clipped_groups(&mut self) -> Result<Option<Out>, Error> {
        if self.clips.is_empty() {
            return Ok(None);
        }
        // `doc` and `clpt` are around the groups.
        let mut out = Out::new(2, self.limits, self.total.clone(), self.counted.clone());
        let mut i = 0;
        // The reader steps through a clipped group's `spnd` and flags, then the lists of its
        // objects, each time a container names the group; each container names its own.
        while let Some(&(id, objects, depth)) = self.clips.get(i) {
            i += 1;
            self.clip_depth = depth;
            self.walk(2)?;
            out.open(b"grp ")?;
            out.chunk(b"spnd", &id.to_le_bytes())?;
            // Byte 2 marks the group a container clips (R67).
            out.chunk(b"flgs", &flags(Flags::default(), 0, 0x80, 0x10))?;
            self.objects(&mut out, objects)?;
            out.close()?;
        }
        Ok(Some(out))
    }

    /// One shape's object list (R19, R22): flags, then its `loda` and its transform in a `lgob`
    /// list. `clip` names the group it clips.
    fn shape(&mut self, out: &mut Out, s: &Shape, clip: Option<u32>) -> Result<(), Error> {
        self.take_object()?;
        // The arguments the shape's `loda` takes, counted before its points are encoded: the
        // outline, the fill and the coordinates, and the winding, transparency and container
        // arguments it carries.
        let opacity = share(s.opacity)?;
        let seen_through = opacity < 1.0 || s.blend != Blend::Normal;
        let arguments = 3 + usize::from(s.nonzero) + usize::from(seen_through) + usize::from(clip.is_some());
        if arguments > self.limits.arguments {
            return Err(Error::Limit("argument count"));
        }
        // The fill and outline records the shape names, added before its points are encoded so
        // that they count toward the limits first.
        let outline = self.outline(s.outline.as_ref())?;
        let fill = self.fill(s.fill.as_ref())?;
        // The transparency record names a white uniform fill as the sample files do (R69).
        let white = if seen_through { Some(self.fill(Some(&Color::Rgb([1.0; 3])))?) } else { None };
        // The object's lists, `obj `, `lgob` and `trfl`, and its chunks, those three with `flgs`,
        // `loda` and `trfd`.
        out.fits(3, 6)?;
        let (kind, coords) = self.geometry(&s.geometry, &s.matrix)?;
        let mut args = vec![(ARG_OUTLINE, outline.to_le_bytes().to_vec()), (ARG_FILL, fill.to_le_bytes().to_vec()), (ARG_COORDS, coords)];
        if s.nonzero {
            args.push((ARG_WINDING, vec![1, 0, 0, 0]));
        }
        // Byte 2 of the flags marks an object seen through (0x08) and a clipping container (0x10)
        // (R67).
        let mut byte2 = 0;
        if let Some(white) = white {
            args.push((ARG_TRANSPARENCY, transparency(opacity, s.blend, white)));
            byte2 |= 0x08;
        }
        if clip.is_some() {
            byte2 |= 0x10;
        }
        if let Some(id) = clip {
            // As sample files lay it out (R46).
            args.push((ARG_CONTAINER, [id, 1, 0, 1].iter().flat_map(|v| v.to_le_bytes()).collect()));
        }
        out.open(b"obj ")?;
        out.chunk(b"flgs", &flags(s.flags, 0, byte2, 0x08))?;
        out.open(b"lgob")?;
        self.loda(out, kind, &args)?;
        out.open(b"trfl")?;
        out.chunk(b"trfd", &trfd(&s.matrix)?)?;
        out.close()?;
        out.close()?;
        out.close()
    }

    /// The object type and coordinates argument of `g`, drawn through `m`.
    fn geometry(&mut self, g: &Geometry, m: &Matrix) -> Result<(u32, Vec<u8>), Error> {
        Ok(match g {
            Geometry::Rect { w, h, radii } => (RECTANGLE, rect(*w, *h, radii, m)?),
            Geometry::Ellipse { w, h } => {
                // Equal start and end angles: a whole ellipse (R17).
                let mut b = [coord(*w)?, coord(*h)?].concat();
                b.extend([0; 12]);
                (ELLIPSE, b)
            }
            Geometry::Path(p) => (CURVE, self.points(p)?),
        })
    }

    /// A curve's point list (R16): the count, the points, then their type bytes.
    fn points(&mut self, path: &Path) -> Result<Vec<u8>, Error> {
        // A subpath without segments draws nothing, and the reader leaves it out.
        let drawn = || path.subpaths.iter().filter(|s| !s.segments.is_empty());
        // A closed subpath ends with a point at its start (R68): a closing line is added when its
        // last segment ends elsewhere.
        let adds_close = |sp: &SubPath| -> Result<bool, Error> {
            let end = match sp.segments.last() {
                Some(Segment::Line(p) | Segment::Cubic(_, _, p)) => *p,
                None => return Ok(false),
            };
            Ok(sp.closed && [coord(end.x)?, coord(end.y)?] != [coord(sp.start.x)?, coord(sp.start.y)?])
        };
        // `n` points within the coordinate count and, at two coordinates and a type byte each, on
        // top of what the file holds so far, within its size.
        let fits = |n: usize| -> Result<(), Error> {
            if self.coordinates.saturating_add(n) > self.limits.coordinates {
                return Err(Error::Limit("coordinate count"));
            }
            fits_stream(&self.limits, self.total.get().saturating_add(n.saturating_mul(9)).saturating_add(4))
        };
        // A subpath takes at least a point for its start and one for each segment, so a path past
        // either limit stops before its segments are looked at one by one, and the count stops
        // at the segment that takes it past.
        let mut n = 0usize;
        for sp in drawn() {
            fits(n.saturating_add(sp.segments.len()).saturating_add(1))?;
            n = n.saturating_add(1);
            for s in &sp.segments {
                n = n.saturating_add(if matches!(s, Segment::Line(_)) { 1 } else { 3 });
                fits(n)?;
            }
            n = n.saturating_add(usize::from(adds_close(sp)?));
            fits(n)?;
        }
        fits(n)?;
        self.coordinates = self.coordinates.saturating_add(n);
        // The record: the count, the points, then the type bytes, which go in their own buffer.
        let mut b = Vec::with_capacity(4 + n * 9);
        b.extend(len32(n)?.to_le_bytes());
        let mut types: Vec<u8> = Vec::with_capacity(n);
        let mut put = |p: Pt, t: u8| -> Result<(), Error> {
            b.extend(coord(p.x)?);
            b.extend(coord(p.y)?);
            types.push(t);
            Ok(())
        };
        for sp in drawn() {
            let added = adds_close(sp)?;
            put(sp.start, EDITABLE | if sp.closed { CLOSES } else { 0 })?;
            let last = sp.segments.len().saturating_sub(1);
            for (i, seg) in sp.segments.iter().enumerate() {
                // The end that closes the subpath is not editable; every other end is (R68).
                let end = if sp.closed && !added && i == last { CLOSES } else { EDITABLE };
                match *seg {
                    Segment::Line(p) => put(p, LINE | end)?,
                    Segment::Cubic(c1, c2, p) => {
                        put(c1, CONTROL)?;
                        put(c2, CONTROL)?;
                        put(p, CURVE_END | end)?;
                    }
                }
            }
            if added {
                put(sp.start, LINE | CLOSES)?;
            }
        }
        b.extend(types);
        Ok(b)
    }

    /// The id of the fill record of `color` (R11): a uniform fill, or a fill of type 0 for none.
    fn fill(&mut self, color: Option<&Color>) -> Result<u32, Error> {
        let mut body = Vec::new();
        match color {
            None => body.extend(0u16.to_le_bytes()),
            Some(c) => {
                let props = properties(c, true)?;
                body.extend(1u16.to_le_bytes());
                body.extend(TAG_1300.to_le_bytes());
                body.extend(len32(props.len())?.to_le_bytes());
                body.extend(props);
                // The 8 bytes that end a uniform fill (R66).
                body.extend([0; 8]);
            }
        }
        let mut record = TAG_1300.to_le_bytes().to_vec();
        record.extend(len32(body.len())?.to_le_bytes());
        record.extend(body);
        // Each fill record sits in a `LIST filc` of its own (R72).
        Self::record(&mut self.fills, &mut self.next_id, record, true, &self.limits, [&self.total, &self.counted])
    }

    /// The id of the outline record of `outline` (R12, R13, R48, R49, R50); without one, an
    /// outline whose line type says none.
    fn outline(&mut self, outline: Option<&Outline>) -> Result<u32, Error> {
        let none = Outline {
            width: 0.0,
            color: Color::Gray(1.0),
            cap: Cap::Butt,
            join: Join::Miter,
            dashes: Vec::new(),
            behind_fill: false,
            scale_with: false,
        };
        let o = outline.unwrap_or(&none);
        // Dash and gap lengths in whole outline widths, as u16, in the room before the arrowhead
        // ids (R12).
        if o.dashes.len() > MAX_DASHES {
            return Err(Error::Unwritable("dash patterns of more than 10 lengths"));
        }
        if o.dashes.iter().any(|d| !(d.is_finite() && *d >= 0.0)) {
            return Err(Error::Unwritable("dash lengths that are negative or not finite"));
        }
        if o.dashes.iter().any(|d| d.round() > f64::from(u16::MAX)) {
            return Err(Error::Unwritable("dash lengths past 65,535 outline widths"));
        }
        let dashes: Vec<u16> = o.dashes.iter().map(|d| d.round() as u16).collect();
        let dashed = outline.is_some() && dashes.iter().any(|d| *d > 0);
        // A pattern whose lengths all round to 0 reads back as a solid line, without its lengths
        // (R48).
        if outline.is_some() && !dashed && !o.dashes.is_empty() {
            return Err(Error::Unwritable("dash patterns whose lengths all round to 0 outline widths"));
        }
        if dashed {
            // The reader counts an outline's dashes for every shape that uses it.
            self.dashes = self.dashes.saturating_add(dashes.len());
            if self.dashes > self.limits.dashes {
                return Err(Error::Limit("dash count"));
            }
        }
        // Bit 0 no outline (R13), 1 as in the plain outlines of sample files (R49), 2 dashed (R48),
        // 4 behind the fill (R49), 5 scaled with the shape (R50).
        let line_type: u16 = match outline {
            None => 1,
            Some(o) => 0x02 | if dashed { 0x04 } else { 0 } | if o.behind_fill { 0x10 } else { 0 } | if o.scale_with { 0x20 } else { 0 },
        };
        let color = color_record(&o.color)?;
        let mut body = Vec::new();
        // A library color is named in a record of type 2 (R12).
        if matches!(o.color, Color::Spot { .. }) {
            // An outline's list ends at its empty property (R71).
            let props = properties(&o.color, false)?;
            body.extend(2u32.to_le_bytes());
            body.extend(len32(props.len())?.to_le_bytes());
            body.extend(props);
        }
        let mut fields = Vec::new();
        for v in [line_type, cap(o.cap), join(o.join)] {
            fields.extend(v.to_le_bytes());
        }
        if o.width < 0.0 {
            return Err(Error::Unwritable("negative outline widths"));
        }
        let width = coord(o.width)?;
        // A width of 0 reads back as no width, which a positive width must not become.
        if o.width > 0.0 && width == [0; 4] {
            return Err(Error::Unwritable("outline widths that round to 0"));
        }
        fields.extend(width);
        // The stretch, 100 percent, and 2 bytes; the nib angle and 46 bytes.
        fields.extend(100u16.to_le_bytes());
        fields.extend([0; 2]);
        fields.extend(angle(0.0)?);
        fields.extend([0; 46]);
        fields.extend(color);
        fields.extend([0; 16]);
        fields.extend((dashes.len() as u16).to_le_bytes());
        let mut slots = [0u8; 2 * MAX_DASHES];
        for (slot, b) in slots.iter_mut().zip(dashes.iter().flat_map(|d| d.to_le_bytes())) {
            *slot = b;
        }
        fields.extend(slots);
        // The start and end arrowhead ids (none), and the rest of the record.
        fields.extend([0; 16]);
        // As sample files hold them (R71): an f64 1 at bytes 16 and 48, and 45 degrees and 60 at
        // bytes 84 and 88.
        let one = 1f64.to_le_bytes();
        let patches: [(usize, &[u8]); 4] = [(16, &one), (48, &one), (84, &45_000_000i32.to_le_bytes()), (88, &60u32.to_le_bytes())];
        for (at, bytes) in patches {
            for (slot, b) in fields.iter_mut().skip(at).zip(bytes) {
                *slot = *b;
            }
        }
        body.extend(1u32.to_le_bytes());
        body.extend(len32(fields.len())?.to_le_bytes());
        body.extend(fields);
        // 8 bytes follow the record (R71).
        body.extend([0; 8]);
        Self::record(&mut self.outlines, &mut self.next_id, body, false, &self.limits, [&self.total, &self.counted])
    }

    /// The id of the record with `body`, adding it when it is new. Its chunk, with its header,
    /// its id and its pad byte, and the list around it when `wrapped`, count toward the file's
    /// bytes and chunks.
    fn record(
        records: &mut Records,
        next_id: &mut u32,
        body: Vec<u8>,
        wrapped: bool,
        limits: &Limits,
        spent: [&Cell<usize>; 2],
    ) -> Result<u32, Error> {
        if let Some(id) = records.ids.get(body.as_slice()) {
            return Ok(*id);
        }
        let [total, counted] = spent;
        let chunk = body.len().saturating_add(4);
        let n = total.get().saturating_add(8).saturating_add(chunk).saturating_add(chunk % 2).saturating_add(if wrapped { 12 } else { 0 });
        fits_stream(limits, n)?;
        let chunks = counted.get().saturating_add(1 + usize::from(wrapped));
        if chunks > limits.chunks {
            return Err(Error::Limit("chunk count"));
        }
        total.set(n);
        counted.set(chunks);
        let id = *next_id;
        *next_id = next_id.checked_add(1).ok_or(Error::Limit("output size"))?;
        let body: Rc<[u8]> = body.into();
        records.bodies.push((id, body.clone()));
        records.ids.insert(body, id);
        Ok(id)
    }
}

/// Whether `objects` hold a shape or a container. Groups that hold only empty groups draw nothing,
/// and the reader leaves them out. Groups nested past the depth are refused, as
/// [`Writer::objects`] refuses them.
fn draws(objects: &[Object], limits: &Limits) -> Result<bool, Error> {
    let mut open = vec![objects.iter()];
    while let Some(level) = open.last_mut() {
        match level.next() {
            None => {
                open.pop();
            }
            Some(Object::Group { objects, .. }) => {
                if open.len() > limits.depth {
                    return Err(Error::Limit("nesting depth"));
                }
                open.push(objects.iter());
            }
            Some(_) => return Ok(true),
        }
    }
    Ok(false)
}

/// Point type bytes (R16): a line end, a control point, a curve end, and bit 3, which closes the
/// subpath; bit 2 marks a node that can be edited (R68).
const LINE: u8 = 0x40;
const CONTROL: u8 = 0xC0;
const CURVE_END: u8 = 0x80;
const CLOSES: u8 = 0x08;
const EDITABLE: u8 = 0x04;

/// A `flgs` chunk (R19): bit 0x10 of byte 0 locks, bit 0x01 of byte 1 hides; byte 3 the record
/// type.
fn flags(f: Flags, byte0: u8, byte2: u8, kind: u8) -> [u8; 4] {
    [byte0 | if f.locked { 0x10 } else { 0 }, u8::from(f.hidden), byte2, kind]
}

/// A length in points as an i32 coordinate (R5).
fn coord(pt: f64) -> Result<[u8; 4], Error> {
    let units = (pt * UNITS_PER_PT).round();
    if !(units.is_finite() && (f64::from(i32::MIN)..=f64::from(i32::MAX)).contains(&units)) {
        return Err(Error::Unwritable("coordinates past the file's range"));
    }
    Ok((units as i32).to_le_bytes())
}

/// A page's width and height as coordinates (R10, R21); the reader takes a size only when both
/// are above 0.
fn page_size((w, h): (f64, f64)) -> Result<Vec<u8>, Error> {
    if !((w * UNITS_PER_PT).round() >= 1.0 && (h * UNITS_PER_PT).round() >= 1.0) {
        return Err(Error::Unwritable("page sizes that are not above 0"));
    }
    Ok([coord(w)?, coord(h)?].concat())
}

/// An angle in degrees as i32 millionths (R7).
fn angle(deg: f64) -> Result<[u8; 4], Error> {
    let fits = |v: f64| (f64::from(i32::MIN)..=f64::from(i32::MAX)).contains(&v);
    let v = (deg * 1_000_000.0).round();
    // An angle past the field's range is written as the same angle within one turn.
    let v = if fits(v) { v } else { (deg.rem_euclid(360.0) * 1_000_000.0).round() };
    if !v.is_finite() {
        return Err(Error::Unwritable("angles that are not finite"));
    }
    Ok((v as i32).to_le_bytes())
}

/// A finite f64, or [`WriteError::Unwritable`].
fn finite(v: f64, what: &'static str) -> Result<[u8; 8], Error> {
    if v.is_finite() { Ok(v.to_le_bytes()) } else { Err(Error::Unwritable(what)) }
}

/// The `mcfg` chunk (R10, R72): the tag 1300, the length of the rest and 4 bytes, the page width
/// and height, then 16 bytes.
fn mcfg(size: (f64, f64)) -> Result<Vec<u8>, Error> {
    let mut b = TAG_1300.to_le_bytes().to_vec();
    b.extend([0; 8]);
    b.extend(page_size(size)?);
    b.extend([0; 16]);
    let rest = len32(b.len() - 8)?.to_le_bytes();
    if let Some(slot) = b.get_mut(4..8) {
        slot.copy_from_slice(&rest);
    }
    Ok(b)
}

/// A `trfd` record of one matrix (R8, R51, R65): length, count, where the offsets lie and 0xFFFF;
/// the offsets with one more for the end; then the entry: the tag 1300 and a u32 0, type 8, 6
/// bytes, `a c tx b d ty` as f64 with the offsets in coordinate units, and 8 bytes.
fn trfd(m: &Matrix) -> Result<Vec<u8>, Error> {
    const ENTRY_AT: u32 = 24;
    const ENTRY_LEN: u32 = 8 + 2 + 6 + 48 + 8;
    let [a, b, c, d, e, f] = m.0;
    let mut out = Vec::with_capacity((ENTRY_AT + ENTRY_LEN) as usize);
    for v in [ENTRY_AT + ENTRY_LEN, 1, 16, 0xFFFF, ENTRY_AT, ENTRY_AT + ENTRY_LEN] {
        out.extend(v.to_le_bytes());
    }
    out.extend(TAG_1300.to_le_bytes());
    out.extend([0; 4]);
    out.extend(8u16.to_le_bytes());
    out.extend([0; 6]);
    for v in [a, c, e * UNITS_PER_PT, b, d, f * UNITS_PER_PT] {
        out.extend(finite(v, "transforms that are not finite")?);
    }
    out.extend([0; 8]);
    Ok(out)
}

/// Rectangle coordinates (R29): width and height in coordinate units as f64, the scale factors
/// 1 and 1, the scale-with flag clear and 7 bytes, then each corner's radius as a share of half the
/// shorter side, 16 bytes apart.
fn rect(w: f64, h: f64, radii: &[f64; 4], m: &Matrix) -> Result<Vec<u8>, Error> {
    // The reader takes sizes within the range of 32-bit coordinates, as stored.
    // Within half a unit of the range is the range, as conversion rounding can carry a size the
    // reader gave just past it.
    let max = f64::from(i32::MAX);
    let stored = |v: f64| -> Result<f64, Error> {
        let u = v * UNITS_PER_PT;
        if !(u.is_finite() && u.abs() <= max + 0.5) {
            return Err(Error::Unwritable("coordinates past the file's range"));
        }
        Ok(u.clamp(-max, max))
    };
    let (wu, hu) = (stored(w)?, stored(h)?);
    let half = w.abs().min(h.abs()) / 2.0;
    // The reader holds a corner to half the shorter side.
    if radii.iter().any(|r| !(r.is_finite() && *r >= 0.0 && *r <= half * (1.0 + 1e-9))) {
        return Err(Error::Unwritable("corner radii below 0 or past half the shorter side"));
    }
    // The scale fields repeat the transform's scale (R29): the lengths its x and y axes take, the
    // writer's own reading for a transform that also turns or skews.
    let [a, b, c, d, ..] = m.0;
    let (sx, sy) = (a.hypot(b), c.hypot(d));
    if !(sx.is_finite() && sy.is_finite()) {
        return Err(Error::Unwritable("transforms that are not finite"));
    }
    let mut out = Vec::with_capacity(32 + 8 + 4 * 8 + 3 * 16);
    for v in [wu, hu, sx, sy] {
        out.extend(v.to_le_bytes());
    }
    out.extend([0; 8]);
    for (k, r) in radii.iter().enumerate() {
        if k > 0 {
            out.extend([0; 16]);
        }
        let share = if half > 0.0 { (r / half).min(1.0) } else { 0.0 };
        out.extend(share.to_le_bytes());
    }
    Ok(out)
}

/// A value from 0 to 1 as a byte from 0 to 255.
fn byte(v: f32) -> Result<u8, Error> {
    Ok((share(v)? * 255.0).round() as u8)
}

/// `v` when it is from 0 to 1, as colors, tints and opacity are.
fn share(v: f32) -> Result<f32, Error> {
    if (0.0..=1.0).contains(&v) { Ok(v) } else { Err(Error::Unwritable("color values, tints or opacity outside 0 to 1")) }
}

/// A 12-byte color record (R14): model, palette, 4 bytes, four value bytes. RGB is model 5 (blue,
/// green, red), CMYK model 3 (0 to 255), gray model 9, registration model 20; a library color is
/// model 21, its full-strength look mixed toward white by its tint and the tint in percent last.
/// A library color without a look is written black.
fn color_record(c: &Color) -> Result<[u8; 12], Error> {
    let (model, v): (u16, [u8; 4]) = match c {
        Color::Rgb([r, g, b]) => (5, [byte(*b)?, byte(*g)?, byte(*r)?, 0]),
        Color::Cmyk([c, m, y, k]) => (3, [byte(*c)?, byte(*m)?, byte(*y)?, byte(*k)?]),
        Color::Gray(ink) => (9, [byte(1.0 - share(*ink)?)?, 0, 0, 0]),
        Color::Registration => (20, [0; 4]),
        Color::Spot { tint, look, .. } => {
            // The tint is stored in whole percent; at 0 the stored color is white whatever the
            // look, which the reader can't take off again.
            let percent = (share(*tint)? * 100.0).round();
            if percent < 1.0 {
                return Err(Error::Unwritable("spot colors at a tint below 0.5 percent"));
            }
            let t = percent / 100.0;
            let [r, g, b] = look.unwrap_or([0.0; 3]);
            // The reader takes the tint off again (`records::untinted`); a low tint leaves too few
            // steps of the stored byte for the look to come back within one step.
            // The nearest byte to the tinted value is written, or the other byte around it when
            // only that one reads back within the step.
            let tinted = |v: f32| -> Result<u8, Error> {
                let ideal = (1.0 - (1.0 - share(v)?) * t) * 255.0;
                let reads_back = |s: &f32| ((1.0 - (1.0 - s / 255.0) / t).clamp(0.0, 1.0) - v).abs() <= 1.0 / 255.0 + 1e-6;
                match [ideal.round(), ideal.floor(), ideal.ceil()].into_iter().find(reads_back) {
                    Some(s) => Ok(s.clamp(0.0, 255.0) as u8),
                    None => Err(Error::Unwritable("spot colors at a tint too low to keep their look")),
                }
            };
            (21, [tinted(b)?, tinted(g)?, tinted(r)?, percent as u8])
        }
    };
    let mut out = [0; 12];
    for (slot, b) in out.iter_mut().zip(model.to_le_bytes().into_iter().chain([0; 6]).chain(v)) {
        *slot = b;
    }
    Ok(out)
}

/// A property list (R11, R12): the color, and for a library color its name in a property of type 8
/// (16 bytes, then the count of UTF-16 units and the units); an empty property ends it. With
/// `tail`, the fields a fill's list has after that follow (R66).
fn properties(c: &Color, tail: bool) -> Result<Vec<u8>, Error> {
    let mut out = vec![1];
    out.extend(12u32.to_le_bytes());
    out.extend(color_record(c)?);
    if let Color::Spot { name, .. } = c {
        // A color without a name reads back as a process color.
        if name.is_empty() {
            return Err(Error::Unwritable("spot colors without a name"));
        }
        let units = units(name)?;
        let mut body = vec![0; 16];
        body.extend(len32(units.len())?.to_le_bytes());
        body.extend(units.iter().flat_map(|u| u.to_le_bytes()));
        out.push(8);
        out.extend(len32(body.len())?.to_le_bytes());
        out.extend(body);
    }
    out.extend([0; 5]);
    if tail {
        // After a fill's list (R66): two u16, the overprint flag (u32), an angle and a u32. The
        // first u16, the angle and the last u32 hold the values the specification names as usual
        // (0, 45 degrees and 60); the second u16 and the flag hold 0, a value sample files hold
        // there.
        out.extend([0; 8]);
        out.extend(45_000_000i32.to_le_bytes());
        out.extend(60u32.to_le_bytes());
    }
    Ok(out)
}

/// A name argument (R9): UTF-16LE and a closing NUL.
fn name(s: &str) -> Result<Vec<u8>, Error> {
    Ok(units(s)?.into_iter().chain([0]).flat_map(u16::to_le_bytes).collect())
}

/// The UTF-16 units of a name the reader reads back whole: at most [`MAX_NAME`] of them, without
/// a NUL, where a name ends.
fn units(s: &str) -> Result<Vec<u16>, Error> {
    let units: Vec<u16> = s.encode_utf16().take(MAX_NAME + 1).collect();
    if units.len() > MAX_NAME || units.contains(&0) {
        return Err(Error::Unwritable("names longer than 256 UTF-16 units or holding a NUL"));
    }
    Ok(units)
}

fn cap(c: Cap) -> u16 {
    match c {
        Cap::Butt => 0,
        Cap::Round => 1,
        Cap::Square => 2,
    }
}

fn join(j: Join) -> u16 {
    match j {
        Join::Miter => 0,
        Join::Round => 1,
        Join::Bevel => 2,
    }
}

/// Argument 8000 of a uniform transparency (R47): a record tagged 1300 whose 117-byte body holds
/// the transparency in percent (f64) at byte 61, the merge mode at byte 77 and the id of `fill` at
/// byte 105 (R69), with the other fields as the sample files hold them (R69).
fn transparency(opacity: f32, blend: Blend, fill: u32) -> Vec<u8> {
    let mut body = [0u8; 117];
    let percent = (1.0 - f64::from(opacity)) * 100.0;
    let fields: [(usize, &[u8]); 6] = [
        (0, &261u32.to_le_bytes()),
        (57, &u32::MAX.to_le_bytes()),
        (61, &percent.to_le_bytes()),
        (69, &100f64.to_le_bytes()),
        (77, &[merge(blend), 0, 0, 0, 2]),
        (105, &fill.to_le_bytes()),
    ];
    for (at, bytes) in fields {
        for (slot, b) in body.iter_mut().skip(at).zip(bytes) {
            *slot = *b;
        }
    }
    let mut out = TAG_1300.to_le_bytes().to_vec();
    out.extend((body.len() as u32).to_le_bytes());
    out.extend(body);
    out
}

/// The merge mode byte of `blend` (R47).
fn merge(blend: Blend) -> u8 {
    match blend {
        Blend::Normal => 0,
        Blend::Multiply => 9,
        Blend::Lighten => 11,
        Blend::Darken => 12,
        Blend::Color => 14,
        Blend::Hue => 15,
        Blend::Saturation => 16,
        Blend::Luminosity => 17,
        Blend::Difference => 24,
        Blend::Screen => 28,
        Blend::Overlay => 29,
        Blend::SoftLight => 30,
        Blend::HardLight => 31,
        Blend::ColorDodge => 33,
        Blend::ColorBurn => 34,
        Blend::Exclusion => 36,
    }
}

/// A ZIP archive (R24) dated 1980-01-01: the stored `mimetype` member (R64), then the RIFF file
/// made of `riff`'s parts, deflated (R25). The parts are compressed as they are, without being
/// joined first, straight into the archive after the member's header. `craft_zip::Writer` streams
/// an entry only with ZIP64 size fields, which `zip.rs` refuses, and otherwise takes one slice.
fn package(riff: &[&[u8]]) -> Result<Vec<u8>, Error> {
    let size = riff.iter().map(|p| p.len()).fold(0usize, usize::saturating_add);
    let mut out = Vec::new();
    let mut dir = Vec::with_capacity(160);
    let at = header(&mut out, MIMETYPE_NAME, 0, crc32fast::hash(MIMETYPE), MIMETYPE.len(), MIMETYPE.len())?;
    out.extend(MIMETYPE);
    entry(&mut dir, &out, at, MIMETYPE_NAME)?;
    // The RIFF file's CRC-32 and packed size go in its header once it is compressed.
    let at = header(&mut out, MEMBER, 8, 0, 0, size)?;
    let start = out.len();
    let mut crc = crc32fast::Hasher::new();
    let mut e = flate2::write::DeflateEncoder::new(out, flate2::Compression::default());
    for part in riff {
        crc.update(part);
        e.write_all(part).map_err(|_| Error::Limit("output size"))?;
    }
    let mut out = e.finish().map_err(|_| Error::Limit("output size"))?;
    let fields = [crc.finalize(), len32(out.len() - start)?].map(u32::to_le_bytes).concat();
    if let Some(slot) = out.get_mut(at + 14..at + 22) {
        slot.copy_from_slice(&fields);
    }
    entry(&mut dir, &out, at, MEMBER)?;
    let dir_at = len32(out.len())?;
    let dir_len = len32(dir.len())?;
    out.extend(dir);
    out.extend(0x0605_4b50u32.to_le_bytes());
    out.extend([0; 4]);
    out.extend(2u16.to_le_bytes());
    out.extend(2u16.to_le_bytes());
    out.extend(dir_len.to_le_bytes());
    out.extend(dir_at.to_le_bytes());
    out.extend(0u16.to_le_bytes());
    Ok(out)
}

/// A member's local header into `out`, stored (`method` 0) or deflated (8), with the CRC-32, the
/// packed size and the full size of its bytes; where the header starts.
fn header(out: &mut Vec<u8>, name: &[u8], method: u16, crc: u32, packed: usize, size: usize) -> Result<usize, Error> {
    let at = out.len();
    out.extend(0x0403_4b50u32.to_le_bytes());
    // Version needed, flags, method, time, date, CRC-32, packed and full sizes, name length.
    for v in [20, 0, method, 0, 0x21] {
        out.extend(v.to_le_bytes());
    }
    for v in [crc, len32(packed)?, len32(size)?] {
        out.extend(v.to_le_bytes());
    }
    out.extend((name.len() as u16).to_le_bytes());
    // No extra field.
    out.extend(0u16.to_le_bytes());
    out.extend(name);
    Ok(at)
}

/// The central directory entry into `dir` of the member whose local header is at `at` in `out`.
fn entry(dir: &mut Vec<u8>, out: &[u8], at: usize, name: &[u8]) -> Result<(), Error> {
    dir.extend(0x0201_4b50u32.to_le_bytes());
    // Made by version 2.0, then the local header's fields from the version needed to the name
    // length.
    dir.extend(20u16.to_le_bytes());
    dir.extend(out.get(at + 4..at + 28).ok_or(Error::Limit("output size"))?);
    // Extra and comment lengths, disk, internal and external attributes, then the header's offset.
    dir.extend([0; 12]);
    dir.extend(len32(at)?.to_le_bytes());
    dir.extend(name);
    Ok(())
}
