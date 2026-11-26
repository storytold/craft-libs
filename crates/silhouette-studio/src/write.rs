//! Writer for Silhouette Studio `.studio3` documents (header `silhouette05;`).
//!
//! It writes a page and shapes made of one or more contours of straight and Bézier segments,
//! with line colour, line width and fill.
//! Every byte is generated from the field layout learned by saving documents in Silhouette
//! Studio 5.0.556 that differ in one thing and comparing them; nothing is copied from
//! Silhouette's files, and every record was checked by opening written files in Silhouette.
//!
//! Only section 0 (the objects) is written: Silhouette Studio opens a document whose other
//! twelve sections are absent.

use crate::Error;

/// A document to write: the page size and the shapes, in millimetres from the page's
/// top-left corner, y down.
#[derive(Debug, Clone, PartialEq)]
pub struct WriteDocument {
    pub page_mm: (f64, f64),
    pub shapes: Vec<WriteShape>,
}

/// One shape: its contours (a compound path has several) and its look.
#[derive(Debug, Clone, PartialEq)]
pub struct WriteShape {
    pub contours: Vec<WriteContour>,
    /// Line colour (R, G, B); `None` is Silhouette's default red cut line.
    pub line_rgb: Option<[u8; 3]>,
    /// Fill colour; `None` is no fill.
    pub fill_rgb: Option<[u8; 3]>,
    /// Line width in millimetres (0 is a hairline).
    pub width_mm: f64,
}

/// One contour: its nodes in order, and whether it closes back to the first.
#[derive(Debug, Clone, PartialEq)]
pub struct WriteContour {
    pub nodes: Vec<WriteNode>,
    pub closed: bool,
}

/// A node and its Bézier handles, as offsets from the node in millimetres: `incoming` shapes
/// the segment arriving here, `outgoing` the one leaving. `None` is a straight segment end.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WriteNode {
    pub x: f64,
    pub y: f64,
    pub incoming: Option<(f64, f64)>,
    pub outgoing: Option<(f64, f64)>,
}

impl WriteNode {
    /// A corner node with no handles.
    pub fn corner(x: f64, y: f64) -> Self {
        Self { x, y, incoming: None, outgoing: None }
    }
}

impl WriteContour {
    /// The contours of `path` (page millimetres, y down) as nodes with handles. Quadratic
    /// segments become their exact cubics; a closing segment that ends on the first node
    /// gives that node its incoming handle.
    pub fn from_path(path: &kurbo::BezPath) -> Vec<WriteContour> {
        use kurbo::PathEl;
        let mut out: Vec<WriteContour> = Vec::new();
        let mut current: Option<WriteContour> = None;
        let mut last = kurbo::Point::ZERO;
        let push_curve = |c: &mut WriteContour, from: kurbo::Point, c1: kurbo::Point, c2: kurbo::Point, to: kurbo::Point| {
            if let Some(prev) = c.nodes.last_mut() {
                prev.outgoing = Some((c1.x - from.x, c1.y - from.y));
            }
            c.nodes.push(WriteNode { x: to.x, y: to.y, incoming: Some((c2.x - to.x, c2.y - to.y)), outgoing: None });
        };
        for el in path.elements() {
            match *el {
                PathEl::MoveTo(p) => {
                    out.extend(current.take());
                    current = Some(WriteContour { nodes: vec![WriteNode::corner(p.x, p.y)], closed: false });
                    last = p;
                }
                PathEl::LineTo(p) => {
                    if let Some(c) = current.as_mut() {
                        c.nodes.push(WriteNode::corner(p.x, p.y));
                    }
                    last = p;
                }
                PathEl::QuadTo(q, p) => {
                    if let Some(c) = current.as_mut() {
                        let c1 = last + (q - last) * (2.0 / 3.0);
                        let c2 = p + (q - p) * (2.0 / 3.0);
                        push_curve(c, last, c1, c2, p);
                    }
                    last = p;
                }
                PathEl::CurveTo(c1, c2, p) => {
                    if let Some(c) = current.as_mut() {
                        push_curve(c, last, c1, c2, p);
                    }
                    last = p;
                }
                PathEl::ClosePath => {
                    if let Some(mut c) = current.take() {
                        c.closed = true;
                        // A last node on top of the first is the same node: fold it in.
                        if c.nodes.len() > 2
                            && let (Some(first), Some(end)) = (c.nodes.first().copied(), c.nodes.last().copied())
                            && (first.x - end.x).hypot(first.y - end.y) < 1e-9
                        {
                            c.nodes.pop();
                            if let Some(first) = c.nodes.first_mut() {
                                first.incoming = end.incoming;
                            }
                        }
                        if let Some(first) = c.nodes.first() {
                            last = kurbo::Point::new(first.x, first.y);
                        }
                        out.push(c);
                    }
                }
            }
        }
        out.extend(current);
        out.retain(|c| c.nodes.len() >= 2);
        out
    }
}

/// The most shapes one written document holds.
pub const MAX_WRITE_SHAPES: usize = 100_000;
/// The most points one written shape holds.
pub const MAX_WRITE_POINTS: usize = 1_000_000;
/// The largest page side, in millimetres.
pub const MAX_PAGE_MM: f64 = 10_000.0;

/// The Silhouette mat the page sits on: 12 × 12 in, its outer edge 20 × 40 mm larger.
const MAT_MM: f64 = 304.8;
/// Silhouette's 10 mm as it stores it (f32 bits 0x411FFFEB, 9.99998).
const TEN_MM: f32 = f32::from_bits(0x411F_FFEB);
/// The constant at 0x3E8 of every file saved here.
const HEADER_MAGIC: u32 = 0x1DCD_672C;

/// Write `doc` as a `.studio3` file. Errors on a page or point that is not finite and
/// positive (pages) or within ±[`MAX_PAGE_MM`], or on too many shapes or points.
pub fn write(doc: &WriteDocument) -> Result<Vec<u8>, Error> {
    write_as(doc, false)
}

/// Write `doc` as a `.studio` (Studio V2, header `silhouette04;`) file, the legacy format
/// Silhouette Studio's Save As offers as "Silhouette Studio V2". It is the `.studio3` objects
/// section without the 1024-byte header, with a shorter page record, no layer objects and the
/// V2 tool defaults. Same checks as [`write`].
pub fn write_v2(doc: &WriteDocument) -> Result<Vec<u8>, Error> {
    write_as(doc, true)
}

fn write_as(doc: &WriteDocument, v2: bool) -> Result<Vec<u8>, Error> {
    let (pw, ph) = doc.page_mm;
    let page_ok = |v: f64| v.is_finite() && v > 0.0 && v <= MAX_PAGE_MM;
    if !(page_ok(pw) && page_ok(ph)) {
        return Err(Error::Malformed(format!("page {pw} × {ph} mm is outside 0–{MAX_PAGE_MM} mm")));
    }
    if doc.shapes.len() > MAX_WRITE_SHAPES {
        return Err(Error::Malformed(format!("more than {MAX_WRITE_SHAPES} shapes")));
    }
    let mut total = 0usize;
    for shape in &doc.shapes {
        if shape.contours.is_empty() {
            return Err(Error::Malformed("a shape needs a contour".to_string()));
        }
        for contour in &shape.contours {
            let n = contour.nodes.len();
            total = total.saturating_add(n);
            if n < 2 || total > MAX_WRITE_POINTS {
                return Err(Error::Malformed(format!("a contour needs 2 nodes, and a document at most {MAX_WRITE_POINTS}")));
            }
            let ok = |v: f64| v.is_finite() && v.abs() <= MAX_PAGE_MM;
            let handle_ok = |h: Option<(f64, f64)>| h.is_none_or(|(x, y)| ok(x) && ok(y));
            if !contour.nodes.iter().all(|n| ok(n.x) && ok(n.y) && handle_ok(n.incoming) && handle_ok(n.outgoing)) {
                return Err(Error::Malformed("a node is not a finite page coordinate".to_string()));
            }
        }
        if !(shape.width_mm.is_finite() && (0.0..=100.0).contains(&shape.width_mm)) {
            return Err(Error::Malformed("line width must be 0 to 100 mm".to_string()));
        }
    }
    let mut w = W::default();
    w.raw(if v2 { b"silhouette04;" } else { b"silhouette05;" });
    root(&mut w, doc, v2);
    let section = w.0;
    if v2 {
        return Ok(section);
    }
    let mut out = vec![0u8; 0x400];
    out[..13].copy_from_slice(b"silhouette05;");
    out[0x10..0x18].copy_from_slice(&(section.len() as u64).to_le_bytes());
    out[0x3E8..0x3EC].copy_from_slice(&HEADER_MAGIC.to_le_bytes());
    out[0x3F0..0x3F8].copy_from_slice(&300_000_000u64.to_le_bytes());
    out[0x3F8..0x400].copy_from_slice(&300_000_000u64.to_le_bytes());
    out.extend_from_slice(&section);
    Ok(out)
}

/// A little-endian byte builder with Silhouette's record framing.
#[derive(Default)]
struct W(Vec<u8>);

impl W {
    fn raw(&mut self, b: &[u8]) {
        self.0.extend_from_slice(b);
    }
    fn u8(&mut self, v: u8) {
        self.0.push(v);
    }
    fn zeros(&mut self, n: usize) {
        self.0.resize(self.0.len() + n, 0);
    }
    fn u32(&mut self, v: u32) {
        self.raw(&v.to_le_bytes());
    }
    /// An f32, rounded from `v` as Silhouette's own conversions are.
    fn f32(&mut self, v: f64) {
        self.raw(&(v as f32).to_le_bytes());
    }
    fn f32s(&mut self, v: &[f64]) {
        v.iter().for_each(|&x| self.f32(x));
    }
    fn rgba(&mut self, c: [f64; 4]) {
        self.f32s(&c);
    }
    /// A length-prefixed string.
    fn s(&mut self, t: &str) {
        self.u32(t.len() as u32);
        self.raw(t.as_bytes());
    }
    /// A record: marker 0x01, a u32 length counting itself, the payload.
    fn rec(&mut self, f: impl FnOnce(&mut W)) {
        self.u8(1);
        self.sized(f);
    }
    /// A record with its own marker byte: object records mark their kind this way (0x01 a
    /// rectangle, 0x02 a polygon, 0x24 a layer, 0x25 the Default Layer).
    fn tagged(&mut self, tag: u8, f: impl FnOnce(&mut W)) {
        self.u8(tag);
        self.sized(f);
    }
    /// A payload prefixed by a u32 length counting itself.
    fn sized(&mut self, f: impl FnOnce(&mut W)) {
        let at = self.0.len();
        self.u32(0);
        f(self);
        let len = (self.0.len() - at) as u32;
        self.0[at..at + 4].copy_from_slice(&len.to_le_bytes());
    }
}

fn rgb(c: [u8; 3]) -> [f64; 4] {
    [f64::from(c[0]) / 255.0, f64::from(c[1]) / 255.0, f64::from(c[2]) / 255.0, 1.0]
}

fn root(w: &mut W, doc: &WriteDocument, v2: bool) {
    w.rec(|w| {
        default_line_style(w);
        page(w, doc.page_mm, v2);
        view(w);
        document(w);
        objects(w, &doc.shapes, v2);
        offset_setting(w);
        cut_layers(w, v2);
        group_all(w);
        default_layer(w);
        flags(w);
        trailer(w);
    });
}

fn default_line_style(w: &mut W) {
    w.rec(|w| {
        w.f32(25.4);
        w.u32(4);
        w.u32(1);
        w.zeros(2);
        w.s("square");
        w.rgba([0.5, 0.5, 0.5, 1.0]);
        w.rgba([0.5, 0.5, 0.5, 1.0]);
    });
}

fn page(w: &mut W, (width, height): (f64, f64), v2: bool) {
    // The page centred on the 12 × 12 in mat; a custom size has no preset name.
    let (x, y) = ((MAT_MM - width) / 2.0, (MAT_MM - height) / 2.0);
    w.rec(|w| {
        w.u32(0);
        w.u8(0);
        w.f32s(&[15.875; 4]);
        w.u32(1);
        w.u32(1);
        w.f32s(&[20.0, 0.5, 2.0, width, height, 1.0, 1.0]);
        w.zeros(12);
        w.f32s(&[x, y]);
        w.u32(0);
        w.rec(|w| {
            w.s("12x12");
            w.f32s(&[324.8, 344.8, 10.0, 20.0]);
            w.u32(0);
            w.u8(1);
            w.f32(10.0);
            w.rgba([159.0 / 255.0, 161.0 / 255.0, 163.0 / 255.0, 1.0]);
            w.zeros(8);
            w.s("12x12");
        });
        w.rec(|w| {
            w.u8(0);
            w.f32(20.0);
        });
        w.rec(|w| {
            w.u8(1);
            w.f32(20.0);
        });
        w.raw(&[0, 0, 1, 0, 0, 0]);
        (0..4).for_each(|_| w.f32(f64::from(TEN_MM)));
        w.u32(0);
        // A V2 page record ends here.
        if v2 {
            return;
        }
        // A page the size of the mat is Silhouette's 12 × 12 preset; others are custom.
        let preset = (width as f32 == MAT_MM as f32 && height as f32 == MAT_MM as f32).then_some("constant:12x12");
        w.s(preset.unwrap_or(""));
        w.rec(|w| {
            w.u8(0);
            w.u32(3);
            w.u8(0);
            w.u32(1);
            w.f32s(&[20.0, 0.5, 100.0]);
            w.u32(0);
            w.f32(f64::from(TEN_MM));
            w.f32(f64::from(TEN_MM));
            // The cut area: the page less 10 mm, computed in f32 as Silhouette does.
            w.f32(f64::from(width as f32) - f64::from(TEN_MM));
            w.f32(f64::from(height as f32) - f64::from(TEN_MM));
            w.s("media");
            for vals in [[25.4, 25.4, 271.6, 184.6], [25.4; 4], [15.875; 4], [25.4, 25.4, 246.2, 159.2]] {
                w.u32(21);
                w.u8(0);
                w.f32s(&vals);
            }
            w.u32(1);
            w.zeros(16);
            w.zeros(3);
            w.u8(1);
            w.u8(0);
        });
        w.zeros(8);
        w.rec(|w| {
            w.f32(322.0);
            w.zeros(8);
            w.f32s(&[-61.0, -40.0]);
            w.u32(0);
        });
        w.u32(0);
        w.raw(&[0, 0, 1]);
        w.rec(|w| {
            w.zeros(16);
            w.u8(0);
        });
        w.rec(|w| {
            w.u32(1);
            w.u32(1);
            w.f32s(&[40.0, 80.0]);
            w.u32(13);
            w.zeros(8);
            w.u8(0);
        });
        w.s("auto");
        w.rec(|w| {
            w.u8(1);
            w.u8(1);
            w.u32(u32::MAX);
        });
        w.u8(1);
        w.rgba([1.0; 4]);
    });
}

fn view(w: &mut W) {
    w.rec(|w| {
        w.f32s(&[0.0, 0.0, 1.0]);
        w.zeros(12);
        w.rec(|w| {
            w.f32(1.0);
            w.u32(3);
            w.u32(0);
            w.f32(1.0);
            w.zeros(16);
        });
        w.u32(2);
        w.u32(0);
    });
}

fn document(w: &mut W) {
    w.rec(|w| {
        w.zeros(5);
        w.rec(|w| {
            w.s("Print Paper");
            w.s("(Light)");
            w.s("PrintPaperLight");
            w.s("Standard light-weight printer or copy paper.");
            w.u32(0);
            w.raw(&[1, 0, 0]);
            w.u32(10);
            w.u32(10);
            w.u8(0);
            w.s("adjustable");
            w.f32(0.1);
            w.u32(0);
        });
        w.zeros(3);
        w.u32(1);
        w.zeros(2);
        w.s("default");
        w.s("Untitled-1");
        w.zeros(2);
        w.f32(3.175);
        w.zeros(9);
        w.u32(2);
        w.f32(10.0);
        w.u8(1);
        w.zeros(7);
        w.f32s(&[1.0, -1.0, -1.0, 4.0, 0.5, 5.0]);
        w.raw(&[0, 1, 1]);
        w.s("edge");
        w.u32(0);
        w.u8(1);
        w.s("cameo5");
        w.u8(1);
        w.s("cameo5");
        w.s("auto");
        w.s("auto");
    });
}

/// A 32-hex-digit id for object `n`, distinct per object and stable across runs.
fn uid(n: u32) -> String {
    format!("{:08x}{:08x}{:08x}{:08x}", 0x5c4a_f700u32 ^ n, n.wrapping_mul(0x9E37_79B9), 0x0c0f_fee0, n)
}

fn objects(w: &mut W, shapes: &[WriteShape], v2: bool) {
    w.rec(|w| {
        if shapes.is_empty() {
            w.u32(u32::MAX);
            return;
        }
        // Count minus one, then (V3) the layers around the shapes, as Silhouette saves them;
        // V2 has no layer objects.
        if v2 {
            w.u32(shapes.len() as u32 - 1);
        } else {
            w.u32(shapes.len() as u32 + 1);
            layer(w, "Layer 1", 1, 0xA7D0, 1);
        }
        for (i, shape) in shapes.iter().enumerate() {
            polygon(w, shape, 0xA800 + i as u32, 100 + i as u32);
        }
        if !v2 {
            layer(w, "Layer 0", 0, 0xA7D5, 2);
        }
    });
}

/// The style record of an object: line colour, line width, the cut style (with its own
/// colour), corner, position and caps, then the fill.
fn style(w: &mut W, line: [f64; 4], width_mm: f64, cut: [f64; 4], fill: Option<[f64; 4]>) {
    w.u8(0);
    w.u8(1);
    w.rgba(line);
    w.u32(0);
    w.f32(width_mm);
    w.rec(|w| {
        w.s("cut");
        w.s("Cut");
        w.s("0");
        w.rgba(cut);
        w.zeros(2);
    });
    w.u8(0);
    w.s("rounded");
    w.u8(1);
    w.s("rounded");
    match fill {
        Some(c) => {
            w.u8(1);
            w.rgba(c);
        }
        None => w.u8(0),
    }
    paint_defaults(w);
    layer_ref(w);
}

/// After the fill: gradient and pattern defaults (identity transforms).
fn paint_defaults(w: &mut W) {
    w.zeros(10);
    w.raw(&[1, 0, 8, 0, 4]);
    w.f32s(&[1.0, 1.0]);
    w.u32(0);
    w.raw(&[0, 1, 0, 1]);
    w.f32s(&[1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]);
    w.raw(&[0, 1]);
    w.f32s(&[1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0]);
    w.zeros(3);
}

/// The reference to the document's Default Layer.
fn layer_ref(w: &mut W) {
    w.rec(|w| w.u32(u32::MAX));
    w.s("Default Layer");
    w.rgba([149.0 / 255.0, 163.0 / 255.0, 169.0 / 255.0, 1.0]);
    w.f32s(&[0.5, 1.5748, 1.143]);
    w.zeros(16);
    w.f32(1.0);
    w.zeros(12);
    w.u32(1);
    w.zeros(9);
    w.f32(1.0);
    w.u32(u32::MAX);
    w.zeros(9);
    w.f32s(&[76.2, 1.0]);
    w.zeros(8);
}

fn inner_tail(w: &mut W) {
    w.u32(0);
    w.u32(50);
    w.zeros(28);
    w.u32(256);
    w.zeros(9);
    w.u32(256);
    w.u32(256);
    w.zeros(14);
}

/// A handle: absent (`00`) or an offset from its node (`01`, f32 dx, dy).
fn handle(w: &mut W, h: Option<(f64, f64)>) {
    match h {
        Some((dx, dy)) => {
            w.u8(1);
            w.f32s(&[dx, dy]);
        }
        None => w.u8(0),
    }
}

/// The exact bounds of a shape's outline (curves included).
fn bounds(shape: &WriteShape) -> kurbo::Rect {
    use kurbo::Shape as _;
    let mut path = kurbo::BezPath::new();
    for c in &shape.contours {
        let mut nodes = c.nodes.iter();
        let Some(first) = nodes.next() else { continue };
        path.move_to((first.x, first.y));
        let mut prev = *first;
        let segment = |path: &mut kurbo::BezPath, a: WriteNode, b: WriteNode| {
            if a.outgoing.is_none() && b.incoming.is_none() {
                path.line_to((b.x, b.y));
            } else {
                let (ox, oy) = a.outgoing.unwrap_or((0.0, 0.0));
                let (ix, iy) = b.incoming.unwrap_or((0.0, 0.0));
                path.curve_to((a.x + ox, a.y + oy), (b.x + ix, b.y + iy), (b.x, b.y));
            }
        };
        for &n in nodes {
            segment(&mut path, prev, n);
            prev = n;
        }
        if c.closed {
            segment(&mut path, prev, *first);
        }
    }
    path.bounding_box()
}

fn polygon(w: &mut W, shape: &WriteShape, ident: u32, id_seed: u32) {
    let b = bounds(shape);
    let (x0, y0, x1, y1) = (b.x0, b.y0, b.x1, b.y1);
    // The visible line colour is the style's first colour; the cut style keeps its own.
    let line = shape.line_rgb.map_or([1.0, 0.0, 0.0, 1.0], rgb);
    let fill = shape.fill_rgb.map(rgb);
    w.rec(|w| {
        w.u8(0);
        w.rec(|w| {
            // The polygon's style, in a record marked 0x02.
            w.tagged(2, |w| {
                w.u8(0);
                w.rec(|w| style(w, line, shape.width_mm, [1.0, 0.6, 0.6, 1.0], fill));
                inner_tail(w);
            });
            // The outline in the object's own coordinates (identity transform): the contour
            // count minus one, then per contour its closed flag, node count minus one and
            // nodes (position, handles, then 0, smooth flag, 0).
            w.rec(|w| {
                w.u32(shape.contours.len() as u32 - 1);
                for c in &shape.contours {
                    w.rec(|w| {
                        w.u8(u8::from(c.closed));
                        w.u32(c.nodes.len() as u32 - 1);
                        for n in &c.nodes {
                            w.rec(|w| {
                                w.f32s(&[n.x, n.y]);
                                handle(w, n.incoming);
                                handle(w, n.outgoing);
                                let smooth = n.incoming.is_some() && n.outgoing.is_some();
                                w.raw(&[0, u8::from(smooth), 0]);
                            });
                        }
                    });
                }
            });
        });
        // Transform (identity), the selection frame, the fill mode.
        w.f32s(&[1.0, 0.0, 0.0, 0.0, 1.0, 0.0]);
        w.zeros(8);
        w.f32s(&[1.0, (x0 + x1) / 2.0, (y0 + y1) / 2.0, (x1 - x0) / 2.0]);
        w.zeros(8);
        w.f32((y1 - y0) / 2.0);
        w.u32(if fill.is_some() { 2 } else { 1 });
        w.u8(0);
        // The page-coordinate outline the reader reads.
        w.rec(|w| {
            w.u32(shape.contours.len() as u32 - 1);
            for c in &shape.contours {
                w.sized(|w| {
                    w.u8(u8::from(c.closed));
                    w.u32(c.nodes.len() as u32 - 1);
                    for n in &c.nodes {
                        w.f32s(&[n.x, n.y]);
                        handle(w, n.incoming);
                        handle(w, n.outgoing);
                    }
                });
            }
        });
        w.u8(1);
        w.u32(ident);
        w.u32(0);
        w.rec(|w| w.zeros(8));
        w.s("Polygon");
        w.u32(0);
        w.s(&uid(id_seed));
        w.u32(0);
        w.u32(8);
        w.u32(0);
        w.u32(u32::MAX);
        w.u32(0);
        w.u32(0);
    });
}

fn layer(w: &mut W, name: &str, index: u32, ident: u32, id_seed: u32) {
    w.rec(|w| {
        w.u8(0);
        w.rec(|w| {
            w.tagged(0x24, |w| {
                w.u8(0);
                w.rec(|w| style(w, [1.0, 0.0, 0.0, 1.0], 0.0, [1.0, 0.6, 0.6, 1.0], None));
                inner_tail(w);
            });
            w.zeros(2);
            w.u32(1);
            w.u32(51);
            w.u32(0x4F9E_8211);
            w.s(&uid(id_seed + 1000));
            w.u8(0);
            w.u32(0x4394_2839);
            w.raw(&[1, 1]);
        });
        w.f32s(&[1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0]);
        w.u32(2);
        w.u8(0);
        w.rec(|w| w.u32(u32::MAX));
        w.u8(1);
        w.u32(ident);
        w.u32(0);
        w.rec(|w| w.zeros(8));
        w.s(name);
        w.u32(index);
        w.s(&uid(id_seed));
        w.zeros(12);
        w.u32(u32::MAX);
        w.zeros(8);
    });
}

fn offset_setting(w: &mut W) {
    w.rec(|w| {
        w.zeros(2);
        w.s("off");
        w.u32(0);
        w.sized(|w| {
            w.u8(0);
            w.f32s(&[10.0; 4]);
        });
        w.u8(0);
    });
}

const PATTERN: &str = "222222222222222222222222222222A";
const MATERIAL_ID: &str = "5bdfacf526573b1fd749d97a05328dbb";

/// A cut layer's tool settings: V2 stores 98 and the action `unknown` where V3 has 100 and
/// `no_action`.
fn tool_block(w: &mut W, v2: bool) {
    w.s("no_tool");
    tool_tail(w, v2);
}

fn tool_tail(w: &mut W, v2: bool) {
    w.u32(if v2 { 98 } else { 100 });
    w.s(MATERIAL_ID);
    w.s(PATTERN);
    w.s("auto");
    w.s(if v2 { "unknown" } else { "no_action" });
}

fn cut_layers(w: &mut W, v2: bool) {
    const LAYERS: [(&str, bool, bool, bool); 10] = [
        ("cnon", false, false, false),
        ("ccut", true, false, false),
        ("cced", true, true, false),
        ("cper", true, false, false),
        ("cped", true, true, false),
        ("ccu2", true, false, true),
        ("cce2", true, true, true),
        ("cpf2", true, false, true),
        ("cpe2", true, true, true),
        ("weed", true, false, false),
    ];
    w.rec(|w| {
        w.u32(4);
        w.u8(0);
        w.u32(LAYERS.len() as u32 - 1);
        for (key, used, edge, tool2) in LAYERS {
            w.s(&format!("layer_@{key}:"));
            w.sized(|w| {
                w.s(&format!("@{key}:"));
                w.s(PATTERN);
                w.raw(&[u8::from(used), 0, u8::from(edge), u8::from(tool2), 0, 0, 0]);
                tool_block(w, v2);
                w.raw(&[u8::from(tool2), 0, 0, 0]);
                w.raw(b"studio4.3extension1_dghyujnbvf");
                w.u8(1);
                tool_tail(w, v2);
                w.raw(&[u8::from(tool2), 0, 0, 0]);
            });
        }
        w.u32(1);
        w.u8(0);
    });
}

fn group_all(w: &mut W) {
    const IDS: [u32; 6] = [1, 2, 3, 5, 6, 7];
    w.rec(|w| {
        w.u32(IDS.len() as u32);
        w.sized(|w| {
            w.u32(4);
            w.u32(0);
            w.s("@all :");
        });
        for id in IDS {
            w.sized(|w| {
                w.u32(id);
                w.u32(u32::MAX);
            });
        }
    });
}

fn default_layer(w: &mut W) {
    let doc_id = uid(0);
    // The tail is built apart: its first 272 bytes close the inner record, the rest follow it.
    let mut tail = W::default();
    default_layer_tail(&mut tail, &doc_id);
    let (inside, outside) = tail.0.split_at(tail.0.len().min(272));
    w.rec(|w| {
        w.u8(0);
        w.rec(|w| {
            w.tagged(0x25, |w| {
                w.u8(0);
                w.rec(|w| style(w, [167.0 / 255.0, 202.0 / 255.0, 1.0, 1.0], 0.5, [1.0, 0.6, 0.6, 1.0], None));
                inner_tail(w);
            });
            w.raw(inside);
        });
        w.raw(outside);
    });
}

fn flags(w: &mut W) {
    w.rec(|w| {
        w.u32(1);
        w.u32(10);
        w.u32(1);
        w.u8(0);
        w.u32(1);
        w.zeros(2);
    });
}

fn preset(w: &mut W, group: &str, n: &str, id: &str, colour: [f64; 4]) {
    w.sized(|w| {
        w.s(group);
        w.s(n);
        w.s(id);
        w.rgba(colour);
        w.rgba(colour);
        w.u32(0);
        w.s(&format!("offsetter.{group}.layer_name.{n}"));
    });
}

fn trailer(w: &mut W) {
    const RED: [f64; 4] = [0.963, 0.0, 0.0, 1.0];
    const BLUE: [f64; 4] = [0.04, 0.424, 1.0, 1.0];
    w.rec(|_| {});
    w.rec(|w| w.zeros(2));
    w.rec(|w| {
        for (group, ids, counts) in [
            ("score", ["92cd7ee5a6fd92e0d8da0a20336d1f86", "2f470615bdc4231752d96cc2e14d6fd1"], [2, 2, 2]),
            ("template", ["57286a9eb74364ccb766fe3f4902d1aa", "2f470615bdc4231752d96cc2e14d6fd1"], [1, 2, 2]),
        ] {
            w.sized(|w| {
                w.s(group);
                preset(w, group, "1", ids[0], RED);
                preset(w, group, "2", ids[1], BLUE);
                w.f32(0.3);
                counts.iter().for_each(|&c| w.u32(c));
                w.u32(1);
                w.u8(0);
            });
        }
    });
    w.rec(|w| w.zeros(8));
    w.zeros(5);
    w.f32(1.27);
    w.rec(|w| w.u8(0));
    w.rec(|w| {
        w.f32(1.5);
        w.raw(&[1, 1, 1]);
    });
    w.rec(|w| {
        w.u8(0);
        w.u32(1);
        w.raw(&[0, 0, 1]);
        w.f32(20.0);
        w.u32(1);
    });
}

/// The Default Layer after its style: its frame, matrices and the document id.
fn default_layer_tail(w: &mut W, doc_id: &str) {
    // (The record before ends with the first two of these four zero bytes.)
    w.zeros(2);
    w.rec(|w| {
        w.u32(4);
        w.u32(28);
        w.u32(0);
        w.u32(0);
        w.f32(50.0);
        w.f32(50.0);
        w.u32(2);
        w.u32(1);
    });
    w.rec(|w| {
        w.u32(4);
        w.rec(|w| {
            w.u32(3);
            w.rec(|w| {
                w.u32(0);
                w.u32(0);
                w.u32(0);
                w.f32(1.0);
            });
            w.rec(|w| {
                w.u32(0);
                w.f32(1.0);
                w.f32(1.0);
                w.f32(1.0);
            });
            w.rec(|w| {
                w.f32(1.0);
                w.f32(1.0);
                w.f32(1.0);
                w.u32(0);
            });
            w.rec(|w| {
                w.f32(1.0);
                w.u32(0);
                w.u32(0);
                w.u32(0);
            });
        });
        w.rec(|w| {
            w.u32(0);
            w.rec(|w| {
                w.u32(0);
                w.u32(0);
                w.u32(0);
                w.f32(1.0);
                w.f32(1.0);
                w.f32(1.0);
                w.f32(1.0);
                w.u32(0);
                w.u8(1);
            });
        });
    });
    w.rec(|w| {
        w.u32(0);
        w.u32(0);
        w.u32(0);
        w.u32(0);
        w.u8(0);
    });
    w.u8(1);
    w.u8(0);
    w.rec(|w| {
        w.u8(0);
        w.f32(25.0);
        w.u8(1);
        w.u8(1);
        w.u8(0);
    });
    w.rec(|w| {
        w.u32(4);
    });
    w.u32(1);
    w.u32(0);
    w.u32(1);
    w.u32(0);
    w.rec(|w| {
        w.u32(0);
        w.u32(0);
        w.u32(0);
        w.u32(0);
        w.u8(0);
    });
    w.f32(1.0);
    w.u32(0);
    w.u32(0);
    w.u32(0);
    w.f32(1.0);
    w.u32(0);
    w.u32(0);
    w.u32(0);
    w.f32(1.0);
    w.f32(0.5);
    w.f32(0.5);
    w.f32(0.5);
    w.u32(0);
    w.u32(0);
    w.f32(0.5);
    w.u32(2);
    w.u8(0);
    w.rec(|w| {
        w.u32(0xffffffff);
    });
    w.rec(|w| {
        w.u32(0);
        w.rec(|w| {
            w.u32(0);
            w.u32(0);
        });
        w.u32(0);
        w.u32(0);
        w.s(doc_id);
        w.u32(0);
        w.u8(0);
        w.u8(0);
    });
    w.u32(0);
    w.u8(0);
    w.u8(0);
    w.u32(0xffffffff);
    w.u32(0);
    w.u32(0);
}

#[cfg(test)]
mod tests {
    use super::*;
    use kurbo::Shape as _;

    fn shape(points: &[(f64, f64)], closed: bool, line: Option<[u8; 3]>) -> WriteShape {
        let nodes = points.iter().map(|&(x, y)| WriteNode::corner(x, y)).collect();
        WriteShape { contours: vec![WriteContour { nodes, closed }], line_rgb: line, fill_rgb: None, width_mm: 0.0 }
    }

    #[test]
    fn written_documents_read_back_with_their_page_shapes_and_colours() {
        let doc = WriteDocument {
            page_mm: (150.0, 120.0),
            shapes: vec![
                shape(&[(25.0, 45.0), (25.0, 105.0), (125.0, 105.0), (125.0, 45.0)], true, None),
                shape(&[(30.0, 110.0), (80.0, 115.0), (130.0, 110.0)], false, Some([0, 90, 200])),
                WriteShape {
                    fill_rgb: Some([250, 200, 40]),
                    width_mm: 0.5,
                    ..shape(&[(60.0, 10.0), (90.0, 35.0), (30.0, 35.0)], true, Some([200, 0, 0]))
                },
            ],
        };
        let bytes = write(&doc).unwrap();
        assert_eq!(bytes.get(..13), Some(b"silhouette05;".as_slice()));
        let back = crate::import(&bytes).unwrap();
        assert_eq!(back.version, crate::Version::V3);
        assert_eq!(back.page_mm, Some((150.0, 120.0)));
        assert_eq!(back.shapes.len(), 3);
        let b = back.shapes[0].path.bounding_box();
        assert!((b.x0 - 25.0).abs() < 1e-4 && (b.y1 - 105.0).abs() < 1e-4, "{b:?}");
        let closes = |i: usize| back.shapes[i].path.elements().iter().any(|e| matches!(e, kurbo::PathEl::ClosePath));
        assert!(closes(0) && !closes(1) && closes(2));
        let lines: Vec<_> = back.shapes.iter().map(|s| s.line_color).collect();
        assert_eq!(lines, [Some([255, 0, 0]), Some([0, 90, 200]), Some([200, 0, 0])]);
    }

    #[test]
    fn curves_and_compound_paths_round_trip() {
        let mut path = kurbo::BezPath::from_svg("M10 50 C10 20 90 20 90 50 C90 80 10 80 10 50 Z M40 45 L60 45 L50 60 Z").unwrap();
        path.apply_affine(kurbo::Affine::IDENTITY);
        let contours = WriteContour::from_path(&path);
        assert_eq!(contours.len(), 2);
        assert_eq!(contours[0].nodes.len(), 2, "the closing curve folds into the first node");
        assert!(contours[0].nodes[0].incoming.is_some() && contours[0].nodes[0].outgoing.is_some());
        let doc = WriteDocument { page_mm: (100.0, 100.0), shapes: vec![WriteShape { contours, line_rgb: None, fill_rgb: None, width_mm: 0.0 }] };
        let back = crate::import(&write(&doc).unwrap()).unwrap();
        assert_eq!(back.shapes.len(), 1, "one compound shape");
        let b = back.shapes[0].path.bounding_box();
        // The ellipse-like curve reaches y = 27.5 at its top, beyond its nodes at y = 50.
        assert!((b.y0 - 27.5).abs() < 0.01 && (b.x0 - 10.0).abs() < 1e-4, "{b:?}");
        let curves = back.shapes[0].path.elements().iter().filter(|e| matches!(e, kurbo::PathEl::CurveTo(..))).count();
        assert_eq!(curves, 2);
    }

    #[test]
    fn studio_v2_documents_read_back() {
        let doc =
            WriteDocument { page_mm: (120.0, 90.0), shapes: vec![shape(&[(10.0, 10.0), (60.0, 10.0), (60.0, 40.0)], true, Some([0, 90, 200]))] };
        let bytes = write_v2(&doc).unwrap();
        assert_eq!(bytes.get(..13), Some(b"silhouette04;".as_slice()));
        let back = crate::import(&bytes).unwrap();
        assert_eq!(back.version, crate::Version::V2);
        assert_eq!(back.page_mm, Some((120.0, 90.0)));
        assert_eq!(back.shapes.len(), 1);
        assert_eq!(back.shapes[0].line_color, Some([0, 90, 200]));
        assert!(write_v2(&WriteDocument { page_mm: (0.0, 1.0), shapes: Vec::new() }).is_err());
    }

    #[test]
    fn an_empty_document_has_the_layout_silhouette_reads() {
        let bytes = write(&WriteDocument { page_mm: (304.8, 304.8), shapes: Vec::new() }).unwrap();
        let section = u64::from_le_bytes(bytes[0x10..0x18].try_into().unwrap()) as usize;
        assert_eq!(bytes.len(), 0x400 + section, "only section 0");
        assert_eq!(&bytes[0x3E8..0x3EC], &HEADER_MAGIC.to_le_bytes());
        let back = crate::import(&bytes).unwrap();
        assert!(back.shapes.is_empty());
        assert_eq!(back.page_mm, Some((304.8, 304.8)));
    }

    #[test]
    fn hostile_documents_are_errors_not_crashes() {
        let ok = shape(&[(0.0, 0.0), (1.0, 1.0)], false, None);
        for doc in [
            WriteDocument { page_mm: (f64::NAN, 10.0), shapes: Vec::new() },
            WriteDocument { page_mm: (0.0, 10.0), shapes: Vec::new() },
            WriteDocument { page_mm: (1e9, 10.0), shapes: Vec::new() },
            WriteDocument { page_mm: (10.0, 10.0), shapes: vec![shape(&[(0.0, 0.0)], false, None)] },
            WriteDocument {
                page_mm: (10.0, 10.0),
                shapes: vec![WriteShape { contours: Vec::new(), ..shape(&[(0.0, 0.0), (1.0, 1.0)], false, None) }],
            },
            WriteDocument { page_mm: (10.0, 10.0), shapes: vec![shape(&[(0.0, f64::INFINITY), (1.0, 1.0)], false, None)] },
            WriteDocument { page_mm: (10.0, 10.0), shapes: vec![WriteShape { width_mm: -1.0, ..ok.clone() }] },
        ] {
            assert!(write(&doc).is_err(), "{doc:?}");
        }
        assert!(write(&WriteDocument { page_mm: (10.0, 10.0), shapes: vec![ok] }).is_ok());
    }
}
