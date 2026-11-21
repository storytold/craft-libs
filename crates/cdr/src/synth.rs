//! Synthetic drawings for tests and fuzz seeds, built from the layouts in `PROVENANCE.md`. Not a
//! `.cdr` writer: nothing checks that other programs open these files.

use std::io::Write as _;

/// A chunk or a list, before it is laid out.
#[derive(Clone, Debug)]
pub enum Node {
    Chunk([u8; 4], Vec<u8>),
    List([u8; 4], Vec<Node>),
}

pub fn chunk(id: &[u8; 4], data: Vec<u8>) -> Node {
    Node::Chunk(*id, data)
}

pub fn list(form: &[u8; 4], children: Vec<Node>) -> Node {
    Node::List(*form, children)
}

fn put(out: &mut Vec<u8>, id: &[u8; 4], size: u32, body: &[u8], len: usize) {
    out.extend(id);
    out.extend(size.to_le_bytes());
    out.extend(body);
    if len % 2 == 1 {
        out.push(0);
    }
}

/// `n` laid out with real sizes.
pub fn plain(n: &Node) -> Vec<u8> {
    let mut out = Vec::new();
    match n {
        Node::Chunk(id, data) => put(&mut out, id, data.len() as u32, data, data.len()),
        Node::List(form, kids) => {
            let mut body = form.to_vec();
            body.extend(kids.iter().flat_map(plain));
            put(&mut out, b"LIST", body.len() as u32, &body, body.len());
        }
    }
    out
}

/// `n` laid out for a compressed stream: each size field is an index into `pool`.
fn pooled(n: &Node, pool: &mut Vec<u32>) -> Vec<u8> {
    let mut out = Vec::new();
    let (id, body) = match n {
        Node::Chunk(id, data) => (*id, data.clone()),
        Node::List(form, kids) => {
            let mut body = form.to_vec();
            for k in kids {
                body.extend(pooled(k, pool));
            }
            (*b"LIST", body)
        }
    };
    let index = pool.len() as u32;
    pool.push(body.len() as u32);
    put(&mut out, &id, index, &body, body.len());
    out
}

fn zlib(data: &[u8]) -> Vec<u8> {
    let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    let _ = e.write_all(data);
    e.finish().unwrap_or_default()
}

/// A `LIST cmpr` holding `children` (R3).
pub fn cmpr(children: &[Node]) -> Node {
    let mut pool = Vec::new();
    let stream: Vec<u8> = children.iter().flat_map(|c| pooled(c, &mut pool)).collect();
    let sizes: Vec<u8> = pool.iter().flat_map(|s| s.to_le_bytes()).collect();
    let block = |raw: &[u8]| {
        let mut b = b"CPng".to_vec();
        b.extend([1, 0, 4, 0]);
        b.extend(zlib(raw));
        b
    };
    let (b1, b2) = (block(&stream), block(&sizes));
    let mut body = Vec::new();
    for (c, u) in [(b1.len(), stream.len()), (b2.len(), sizes.len())] {
        body.extend((c as u32).to_le_bytes());
        body.extend((u as u32).to_le_bytes());
    }
    body.extend(b1);
    body.extend(b2);
    Node::Chunk(*b"LIST", [b"cmpr".to_vec(), body].concat())
}

/// The form type letter of `version` (hundreds) (R1).
pub fn form(version: u16) -> [u8; 4] {
    let n = (version / 100) as u8;
    let c = match n {
        // Version 3 has a space where later versions have their character.
        3 => b' ',
        0..=9 => b'0' + n,
        10..=17 => b'A' + n - 10,
        _ => b'J' + n - 18,
    };
    [b'C', b'D', b'R', c]
}

/// A RIFF file of `version` holding `children`.
pub fn riff(version: u16, children: &[Node]) -> Vec<u8> {
    let mut body = form(version).to_vec();
    for c in children {
        body.extend(match c {
            // A prebuilt `LIST cmpr` carries its own form type.
            Node::Chunk(id, data) if id == b"LIST" => {
                let mut o = Vec::new();
                put(&mut o, id, data.len() as u32, data, data.len());
                o
            }
            _ => plain(c),
        });
    }
    let mut out = b"RIFF".to_vec();
    out.extend((body.len() as u32).to_le_bytes());
    out.extend(body);
    out
}

/// Field writers for one version.
#[derive(Clone, Copy, Debug)]
pub struct V(pub u16);

impl V {
    pub fn short(self) -> bool {
        self.0 < 600
    }

    pub fn word(self, v: usize) -> Vec<u8> {
        if self.short() { (v as u16).to_le_bytes().to_vec() } else { (v as u32).to_le_bytes().to_vec() }
    }

    /// Rectangle coordinates of `w` × `h` points with corners of `radius` points: coordinates
    /// before version 15 (R15), from 15 f64 sizes and radii as shares of half the shorter side
    /// (R29).
    pub fn rect(self, w: f64, h: f64, radius: f64) -> Vec<u8> {
        if self.0 < 1500 {
            let n = if self.0 >= 900 { 4 } else { 1 };
            let mut out = [self.coord(w), self.coord(h)].concat();
            for _ in 0..n {
                out.extend(self.coord(radius));
            }
            return out;
        }
        let units = |pt: f64| pt / 72.0 * 254_000.0;
        let mut out = Vec::new();
        for v in [units(w), units(h), 1.0, 1.0] {
            out.extend(v.to_le_bytes());
        }
        out.extend([0; 8]);
        let share = radius / (w.abs().min(h.abs()) / 2.0);
        for k in 0..4 {
            if k > 0 {
                out.extend([0; 16]);
            }
            out.extend(share.to_le_bytes());
        }
        out
    }

    /// A length in points as a coordinate.
    pub fn coord(self, pt: f64) -> Vec<u8> {
        if self.short() {
            ((pt / 72.0 * 1000.0).round() as i16).to_le_bytes().to_vec()
        } else {
            ((pt / 72.0 * 254_000.0).round() as i32).to_le_bytes().to_vec()
        }
    }

    pub fn angle(self, deg: f64) -> Vec<u8> {
        if self.short() { ((deg * 10.0).round() as i16).to_le_bytes().to_vec() } else { ((deg * 1_000_000.0).round() as i32).to_le_bytes().to_vec() }
    }

    /// A `loda` record of object type `kind` (R6): header, arguments, offsets, then the types in
    /// reverse.
    pub fn loda(self, kind: u32, args: &[(u32, Vec<u8>)]) -> Vec<u8> {
        let w = if self.short() { 2 } else { 4 };
        let head = 5 * w;
        let mut body: Vec<u8> = Vec::new();
        let mut offsets = Vec::new();
        for (_, a) in args {
            offsets.push(head + body.len());
            body.extend(a);
        }
        offsets.push(head + body.len());
        let offsets_at = head + body.len();
        let types_at = offsets_at + offsets.len() * w;
        let total = types_at + args.len() * w;
        let mut out = Vec::new();
        for f in [total, args.len(), offsets_at, types_at, kind as usize] {
            out.extend(self.word(f));
        }
        out.extend(body);
        for o in offsets {
            out.extend(self.word(o));
        }
        for (t, _) in args.iter().rev() {
            out.extend(self.word(*t as usize));
        }
        out
    }

    /// A `trfd` of `matrices` `[a, b, c, d, e, f]` with offsets in points (R8); before version 5
    /// each matrix is in fixed point (R37).
    pub fn trfd(self, matrices: &[[f64; 6]]) -> Vec<u8> {
        let w = if self.short() { 2 } else { 4 };
        let head = 3 * w;
        let table = head;
        let mut entries = Vec::new();
        let mut offsets = Vec::new();
        // The table holds one offset more than the entries: the record's end (R51).
        let first = table + (matrices.len() + 1) * w;
        for m in matrices {
            offsets.push(first + entries.len());
            if self.0 >= 1300 {
                entries.extend([0; 8]);
            }
            entries.extend(8u16.to_le_bytes());
            if !self.short() {
                entries.extend([0; 6]);
            }
            if self.0 < 500 {
                entries.extend(fixed_matrix(m));
                continue;
            }
            let unit = if self.short() { 1000.0 / 72.0 } else { 254_000.0 / 72.0 };
            let [a, b, c, d, e, f] = *m;
            for v in [a, c, e * unit, b, d, f * unit] {
                entries.extend(v.to_le_bytes());
            }
        }
        offsets.push(first + entries.len());
        let mut out = Vec::new();
        for f in [first + entries.len(), matrices.len(), table] {
            out.extend(self.word(f));
        }
        for o in offsets {
            out.extend(self.word(o));
        }
        out.extend(entries);
        out
    }

    /// A curve's point list (R16).
    pub fn points(self, pts: &[(f64, f64, u8)]) -> Vec<u8> {
        let mut out = (pts.len() as u32).to_le_bytes().to_vec();
        for (x, y, _) in pts {
            out.extend(self.coord(*x));
            out.extend(self.coord(*y));
        }
        out.extend(pts.iter().map(|p| p.2));
        out
    }

    /// A uniform fill record (R11) of 12-byte color `color`.
    pub fn fill(self, id: u32, color: &[u8; 12]) -> Vec<u8> {
        let mut out = id.to_le_bytes().to_vec();
        if self.0 >= 1300 {
            let mut props = vec![1];
            props.extend(12u32.to_le_bytes());
            props.extend(color);
            props.extend([0, 0, 0, 0, 0]);
            let mut solid = 1300u32.to_le_bytes().to_vec();
            solid.extend((props.len() as u32).to_le_bytes());
            solid.extend(props);
            let mut body = 1u16.to_le_bytes().to_vec();
            body.extend(solid);
            out.extend(1300u32.to_le_bytes());
            out.extend((body.len() as u32).to_le_bytes());
            out.extend(body);
        } else {
            out.extend(1u16.to_le_bytes());
            out.extend([0, 0]);
            out.extend(color);
        }
        out
    }

    /// An outline record (R12) of `pen`, `color` and `dashes`.
    pub fn outline(self, id: u32, pen: Pen, color: &[u8; 12], dashes: &[u16]) -> Vec<u8> {
        let Pen { line_type, width, cap, join } = pen;
        let mut out = id.to_le_bytes().to_vec();
        if self.0 >= 1300 {
            out.extend(1u32.to_le_bytes());
            out.extend(0u32.to_le_bytes());
        }
        for f in [line_type, cap, join] {
            out.extend(f.to_le_bytes());
        }
        let v = self.0;
        out.extend(vec![0; if (600..1300).contains(&v) { 2 } else { 0 }]);
        out.extend(self.coord(width));
        out.extend(100u16.to_le_bytes());
        out.extend(vec![0; if self.short() { 0 } else { 2 }]);
        out.extend(self.angle(0.0));
        out.extend(vec![
            0;
            if v >= 1300 {
                46
            } else if self.short() {
                0
            } else {
                52
            }
        ]);
        out.extend(color);
        out.extend(vec![0; if self.short() { 10 } else { 16 }]);
        out.extend((dashes.len() as u16).to_le_bytes());
        for d in dashes {
            out.extend(d.to_le_bytes());
        }
        out.extend([0; 32]);
        out
    }
}

/// An outline's line type (bit 0: none), width in points, cap and join.
#[derive(Clone, Copy, Debug)]
pub struct Pen {
    pub line_type: u16,
    pub width: f64,
    pub cap: u16,
    pub join: u16,
}

/// A 12-byte color record (R14) of `model` and its four value bytes.
pub fn color(model: u16, v: [u8; 4]) -> [u8; 12] {
    let mut c = [0; 12];
    c[..2].copy_from_slice(&model.to_le_bytes());
    c[8..].copy_from_slice(&v);
    c
}

/// A version 4 color (R36): `model` and its four values as u16, then 2 bytes.
pub fn color4(model: u16, v: [u16; 4]) -> [u8; 12] {
    let mut c = [0; 12];
    c[..2].copy_from_slice(&model.to_le_bytes());
    for (k, x) in v.into_iter().enumerate() {
        c[2 + 2 * k..4 + 2 * k].copy_from_slice(&x.to_le_bytes());
    }
    c
}

/// A color of versions 1 to 3 (R36): `model` and its four value bytes.
pub fn color3(model: u8, v: [u8; 4]) -> [u8; 5] {
    [model, v[0], v[1], v[2], v[3]]
}

/// A matrix `[a, b, c, d, e, f]` with offsets in points as six i32 `a c tx b d ty`: the factors in
/// 16.16 fixed point, the offsets in 1/1000 inch (R37).
fn fixed_matrix(m: &[f64; 6]) -> Vec<u8> {
    let [a, b, c, d, e, f] = *m;
    let fixed = |v: f64| (v * 65536.0).round() as i32;
    let units = |pt: f64| (pt / 72.0 * 1000.0).round() as i32;
    [fixed(a), fixed(c), units(e), fixed(b), fixed(d), units(f)].iter().flat_map(|v| v.to_le_bytes()).collect()
}

/// The transforms argument (100) of a version 3 object or group (R33): one matrix entry for each
/// of `matrices` `[a, b, c, d, e, f]` with offsets in points, applied in the order given.
pub fn transforms3(matrices: &[[f64; 6]]) -> Vec<u8> {
    let first = 4 + 10 * matrices.len();
    let mut out = vec![0, 0];
    out.extend((matrices.len() as u16).to_le_bytes());
    for i in 0..matrices.len() {
        out.extend(0x8cu16.to_le_bytes());
        out.extend([0; 4]);
        out.extend(((first + 24 * i) as u16).to_le_bytes());
        out.extend([0; 2]);
    }
    out.extend(matrices.iter().flat_map(fixed_matrix));
    out
}

/// The fill argument (20) of a version 3 object (R34): the type byte, then `color` for a uniform
/// fill (type 1).
pub fn fill3(kind: u8, color: &[u8]) -> Vec<u8> {
    let mut out = vec![kind];
    out.extend(color);
    out
}

/// The outline argument (10) of a version 3 object (R35) of `pen`, `color` and `dashes`; bit 0 of
/// its flags is set unless the pen's line type says no outline.
pub fn outline3(pen: Pen, color: [u8; 5], dashes: &[u8]) -> Vec<u8> {
    let v = V(300);
    let mut out = vec![u8::from(pen.line_type & 1 == 0)];
    out.extend(v.coord(pen.width));
    out.extend(100u16.to_le_bytes());
    out.extend(v.angle(0.0));
    out.extend(color);
    out.extend([0; 7]);
    out.push(dashes.len() as u8);
    let mut area = dashes.to_vec();
    area.resize(10, 0);
    out.extend(area);
    out.extend(pen.join.to_le_bytes());
    out.extend(pen.cap.to_le_bytes());
    out.extend([0; 8]);
    out
}

/// A version 3 group (R33): its own `lobj` with the transforms that place `objects`, then the
/// objects.
pub fn group3(matrices: &[[f64; 6]], objects: Vec<Node>) -> Node {
    let mut kids = vec![chunk(b"flgs", vec![0, 0, 0, 0x10]), chunk(b"lobj", V(300).loda(0, &[(100, transforms3(matrices))]))];
    kids.extend(objects);
    list(b"grp ", kids)
}

/// An object list: flags, then a `lgob` list with the `loda` and an optional transform in a `trfl`
/// list (R22). A version 3 object is a `lobj` whose type counts from 0 (R32) and whose transform is
/// its argument 100 (R33).
pub fn object(v: V, kind: u32, args: &[(u32, Vec<u8>)], matrix: Option<[f64; 6]>) -> Node {
    if v.0 < 400 {
        let kind = if (1..=5).contains(&kind) { kind - 1 } else { kind };
        let mut args = args.to_vec();
        args.extend(matrix.map(|m| (100, transforms3(&[m]))));
        return list(b"obj ", vec![chunk(b"flgs", vec![0, 0, 0, 0x08]), chunk(b"lobj", v.loda(kind, &args))]);
    }
    let mut own = vec![chunk(b"loda", v.loda(kind, args))];
    if let Some(m) = matrix {
        own.push(list(b"trfl", vec![chunk(b"trfd", v.trfd(&[m]))]));
    }
    list(b"obj ", vec![chunk(b"flgs", vec![0, 0, 0, 0x08]), list(b"lgob", own)])
}

/// A layer list of `kind` flags named `name` holding `objects`, its `loda` in a `lgob` list (R22),
/// with the object type that layers of such flags have in sample files (R19).
pub fn layer(v: V, flags: [u8; 4], name: &str, objects: Vec<Node>) -> Node {
    let kind = match flags {
        [b0, ..] if (b0 & 0x0f == 0x0a || b0 & 0x0f == 0x02) && b0 & 0x10 != 0 => 11,
        [b0, ..] if b0 & 0x0f == 0x0a || b0 & 0x0f == 0x02 => 12,
        [b0, _, b2, _] if b0 & 0x0f == 0x08 && b2 != 0 => 17,
        [0, 1, 1, _] => 17,
        _ => 0,
    };
    // Version 3 counts its types from 0, one less (R32).
    let kind = if v.0 < 400 && kind > 0 { kind - 1 } else { kind };
    layer_of_type(v, flags, kind, name, objects)
}

/// A layer list as [`layer`] writes it, with the object type `kind` in its `loda`.
pub fn layer_of_type(v: V, flags: [u8; 4], kind: u32, name: &str, objects: Vec<Node>) -> Node {
    let name: Vec<u8> =
        if v.0 >= 1200 { name.encode_utf16().chain([0]).flat_map(|u| u.to_le_bytes()).collect() } else { name.bytes().chain([0]).collect() };
    let mut kids = vec![chunk(b"flgs", flags.to_vec()), list(b"lgob", vec![chunk(b"loda", v.loda(kind, &[(1000, name)]))])];
    kids.extend(objects);
    list(b"layr", kids)
}

/// A page list: flags (byte 2 set for the master page), then its layers in a `gobj` list (R22).
pub fn page(master: bool, layers: Vec<Node>) -> Node {
    list(b"page", vec![chunk(b"flgs", vec![0, 0, u8::from(master), 0x90]), list(b"gobj", layers)])
}

/// The `mcfg` chunk of `version` with a page of `w` × `h` points (R10); in version 3, 2 bytes and
/// the page's top left and bottom right corners (R38).
pub fn mcfg(v: V, w: f64, h: f64) -> Node {
    if v.0 < 400 {
        let mut b = vec![0; 2];
        b.extend([v.coord(-w / 2.0), v.coord(h / 2.0), v.coord(w / 2.0), v.coord(-h / 2.0)].concat());
        b.extend([0; 16]);
        return chunk(b"mcfg", b);
    }
    let skip = if v.0 >= 1300 {
        12
    } else if v.0 >= 900 {
        4
    } else if (600..700).contains(&v.0) {
        28
    } else {
        0
    };
    let mut b = vec![0; skip];
    b.extend(v.coord(w));
    b.extend(v.coord(h));
    b.extend([0; 16]);
    chunk(b"mcfg", b)
}

/// A style of a [`stlt_body`]: its id, its parent's id, and its fill and outline table entries.
#[derive(Clone, Copy, Debug)]
pub struct Style {
    pub id: u32,
    pub parent: u32,
    pub fill: u32,
    pub outline: u32,
}

/// The body of a `LIST stlt` of `v` (R45): the style count, the fill table (`(entry, fild id)`) and
/// the outline table (`(entry, outl id)`), the text style tables (each with one record when
/// `text`), then `styles` as styles of kind 1, and when `text` a style of kind 2 and one of kind 3.
pub fn stlt_body(v: V, fills: &[(u32, u32)], outlines: &[(u32, u32)], styles: &[Style], text: bool) -> Vec<u8> {
    let version = v.0;
    let u32s = |vals: &[u32]| -> Vec<u8> { vals.iter().flat_map(|x| x.to_le_bytes()).collect() };
    let count = styles.len() + if text { 2 } else { 0 };
    let mut b = u32s(&[count as u32]);
    // A table without styles holds nothing else.
    if count == 0 {
        return b;
    }
    for (table, size) in [(fills, if version >= 1300 { 60 } else { 12 }), (outlines, 12)] {
        b.extend(u32s(&[table.len() as u32]));
        for (entry, record) in table {
            let mut e = u32s(&[*entry, 0, *record]);
            e.resize(size, 0);
            b.extend(e);
        }
    }
    let n = u32::from(text);
    let fixed = |size: usize| -> Vec<u8> {
        let mut t = n.to_le_bytes().to_vec();
        t.extend(vec![0x5a; size * n as usize]);
        t
    };
    // Fonts, alignments, spacings and two tables of fixed records.
    for size in [if version >= 1000 { 60 } else { 44 }, 12, 52, 152, 784] {
        b.extend(fixed(size));
    }
    // One bullet, whose u32 inside asks for the longer form.
    b.extend(n.to_le_bytes());
    if text {
        b.extend([0x5a; 40]);
        if version >= 1300 {
            if version >= 1400 {
                b.extend([0x5a; 4]);
            }
            b.extend(u32s(&[1]));
            b.extend([0x5a; 68]);
        } else {
            b.extend(vec![0x5a; if version >= 1000 { 28 } else { 20 }]);
            b.extend(u32s(&[1]));
            b.extend([0x5a; 16]);
        }
    }
    // Indents, hyphenation, drop caps, and from version 8.01 one more table.
    for size in [28, if version >= 1300 { 36 } else { 32 }, 28] {
        b.extend(fixed(size));
    }
    if version >= 801 {
        b.extend(fixed(12));
    }
    let text_styles = [(2, Style { id: 0x7f02, parent: 0, fill: 0, outline: 0 }), (3, Style { id: 0x7f03, parent: 0x7f02, fill: 0, outline: 0 })];
    let all = styles.iter().map(|s| (1u32, *s)).chain(text_styles.into_iter().filter(|_| text));
    for (kind, s) in all {
        b.extend(u32s(&[kind, s.id, s.parent, 1, 1]));
        if version >= 1200 {
            b.extend(u32s(&[7]));
            b.extend("Graphic".encode_utf16().flat_map(u16::to_le_bytes));
        } else {
            b.extend(u32s(&[8]));
            b.extend(b"Graphic\0");
        }
        b.extend(u32s(&[s.fill, s.outline]));
        if kind > 1 {
            b.extend(vec![0; if version >= 801 { 20 } else { 16 }]);
        }
        if kind > 2 {
            b.extend([0; 20]);
        }
    }
    b
}

/// A `LIST stlt` of [`stlt_body`].
pub fn stlt(v: V, fills: &[(u32, u32)], outlines: &[(u32, u32)], styles: &[Style], text: bool) -> Node {
    Node::Chunk(*b"LIST", [b"stlt".to_vec(), stlt_body(v, fills, outlines, styles, text)].concat())
}

/// A `LIST clpt` of clipped groups (R46): each a `LIST grp ` with its `spnd` id, then its objects.
pub fn clpt(groups: Vec<(u32, Vec<Node>)>) -> Node {
    let groups = groups
        .into_iter()
        .map(|(id, objects)| {
            let mut kids = vec![chunk(b"spnd", id.to_le_bytes().to_vec())];
            kids.extend(objects);
            list(b"grp ", kids)
        })
        .collect();
    list(b"clpt", groups)
}

/// The container argument (8005) naming clipped group `id` (R46), laid out as sample files lay it
/// out.
pub fn container(id: u32) -> (u32, Vec<u8>) {
    (8005, [id, 1, 0, 1].iter().flat_map(|v| v.to_le_bytes()).collect())
}

/// The argument 8000 of a uniform transparency from version 13 (R47): one record of version 1300
/// whose 117-byte body holds `percent` at byte 61 and `merge` at byte 77.
pub fn transparency(percent: f64, merge: u8) -> (u32, Vec<u8>) {
    let mut body = vec![0; 117];
    body[61..69].copy_from_slice(&percent.to_le_bytes());
    body[77] = merge;
    let mut out = 1300u32.to_le_bytes().to_vec();
    out.extend((body.len() as u32).to_le_bytes());
    out.extend(body);
    (8000, out)
}

/// A property list (R11, R12) of 12-byte color `color` and, when `name` is given, a type 8
/// property with that library name.
pub fn properties(color: &[u8; 12], name: Option<&str>) -> Vec<u8> {
    let mut out = vec![1];
    out.extend(12u32.to_le_bytes());
    out.extend(color);
    if let Some(name) = name {
        let mut body = vec![0; 16];
        body.extend((name.encode_utf16().count() as u32).to_le_bytes());
        body.extend(name.encode_utf16().flat_map(u16::to_le_bytes));
        out.push(8);
        out.extend((body.len() as u32).to_le_bytes());
        out.extend(body);
    }
    out.extend([0, 0, 0, 0, 0]);
    out
}

/// `n` (an object, group or layer list) with its `flgs` chunk replaced by `flags` (R19).
pub fn flagged(n: Node, flags: [u8; 4]) -> Node {
    match n {
        Node::List(form, kids) => {
            let kids = kids
                .into_iter()
                .map(|k| if matches!(&k, Node::Chunk(id, _) if id == b"flgs") { chunk(b"flgs", flags.to_vec()) } else { k })
                .collect();
            Node::List(form, kids)
        }
        chunk => chunk,
    }
}

/// A RIFF drawing of `version` (7 to 13) with a style table, a clipping container and a lens: a rectangle
/// that takes its fill and outline from its style, a container that clips two rectangles, one
/// with an outline behind the fill, a rectangle with a lens, a hidden and locked rectangle with a
/// winding flag and a uniform transparency, and a hidden group whose rectangle's outline names a
/// library color from version 13; compressed when `compress`. Before version 7, the [`sample`].
pub fn feature_sample(version: u16, compress: bool) -> Vec<u8> {
    if version < 700 {
        return sample(version, false);
    }
    let v = V(version);
    let black = color(2, [0, 0, 0, 100]);
    let rect = |w: f64, extra: Vec<(u32, Vec<u8>)>, at: f64| {
        let mut args = vec![(30, v.rect(w, w, 0.0))];
        args.extend(extra);
        object(v, 1, &args, Some([1.0, 0.0, 0.0, 1.0, at, at]))
    };
    let style = (200, 0x10u32.to_le_bytes().to_vec());
    let clipped =
        vec![rect(30.0, vec![style.clone()], 10.0), rect(30.0, vec![(10, 3u32.to_le_bytes().to_vec()), (20, 1u32.to_le_bytes().to_vec())], -10.0)];
    // From version 13 outline 4 names a library color in a record of type 2 (R12).
    let mut named = v.outline(4, Pen { line_type: 0, width: 1.0, cap: 0, join: 0 }, &color(25, [7, 0, 100, 0]), &[]);
    if version >= 1300 {
        let props = properties(&color(5, [0, 0, 255, 0]), Some("Sky Blue"));
        let mut record = 2u32.to_le_bytes().to_vec();
        record.extend((props.len() as u32).to_le_bytes());
        record.extend(props);
        named.splice(4..4, record);
    }
    let doc = list(
        b"doc ",
        vec![
            mcfg(v, 612.0, 792.0),
            list(b"filt", vec![chunk(b"fild", v.fill(1, &color(5, [0, 0, 255, 0])))]),
            list(
                b"otlt",
                vec![
                    chunk(b"outl", v.outline(2, Pen { line_type: 4, width: 2.0, cap: 1, join: 1 }, &black, &[3, 1])),
                    chunk(b"outl", v.outline(3, Pen { line_type: 0x12, width: 4.0, cap: 0, join: 0 }, &black, &[])),
                    chunk(b"outl", named),
                ],
            ),
            stlt(v, &[(0x100, 1)], &[(0x200, 2)], &[Style { id: 0x10, parent: 0, fill: 0x100, outline: 0x200 }], true),
            clpt(vec![(7, clipped)]),
        ],
    );
    let objects = vec![
        rect(20.0, vec![style.clone()], 0.0),
        rect(40.0, vec![style.clone(), container(7)], 50.0),
        rect(20.0, vec![style.clone(), (8000, vec![1, 0, 0, 0])], -50.0),
        // Hidden and locked, filled nonzero, 25% transparent with merge mode 9 (R19, R47, R52).
        flagged(rect(10.0, vec![style.clone(), (14001, vec![1, 0, 0, 0]), transparency(25.0, 9)], 80.0), [0x10, 1, 0, 0x08]),
        list(b"grp ", vec![chunk(b"flgs", vec![0, 1, 0, 0x10]), rect(10.0, vec![(10, 4u32.to_le_bytes().to_vec())], -80.0)]),
    ];
    let page = page(false, vec![layer(v, [0, 0, 0, 0x98], "Art", objects)]);
    let mut body = vec![chunk(b"vrsn", version.to_le_bytes().to_vec())];
    if compress {
        body.push(cmpr(&[doc, page]));
    } else {
        body.extend([doc, page]);
    }
    riff(version, &body)
}

/// A file of `version` holding `content` as the program of that version saves it: a RIFF file
/// before 14, then a package (R25, R26).
pub fn file_of(version: u16, content: Vec<Node>) -> Vec<u8> {
    let mut body = vec![chunk(b"vrsn", version.to_le_bytes().to_vec())];
    body.extend(content);
    match version {
        ..1400 => riff(version, &body),
        1400..1600 => package14(riff(version, &body), false),
        _ => package16(version, &body, false, &[]),
    }
}

/// The document list and pages of the [`sample`] of `version`.
pub fn sample_content(version: u16) -> Vec<Node> {
    let v = V(version);
    // Before version 5 the sample's colors are CMYK in percent, model 1 (R36).
    let (red, black) =
        if version < 500 { (color4(1, [0, 100, 100, 0]), color4(1, [0, 0, 0, 100])) } else { (color(5, [0, 0, 255, 0]), color(2, [0, 0, 0, 100])) };
    let (round, none) = (Pen { line_type: 0, width: 2.0, cap: 1, join: 1 }, Pen { line_type: 1, width: 1.0, cap: 0, join: 0 });
    let mut settings = vec![mcfg(v, 612.0, 792.0)];
    // Version 3 keeps fills and outlines in the objects (R34, R35).
    if version >= 400 {
        settings.push(list(b"filt", vec![chunk(if version >= 700 { b"fild" } else { b"fill" }, v.fill(1, &red))]));
        settings.push(list(b"otlt", vec![chunk(b"outl", v.outline(2, round, &black, &[])), chunk(b"outl", v.outline(3, none, &black, &[]))]));
    }
    let doc = list(b"doc ", settings);
    // Fill 1 is red and 0 none; outline 2 is 2 pt with round caps and joins, 3 none.
    let style = |fill: u32, outline: u32| {
        if version >= 400 {
            return vec![(20, fill.to_le_bytes().to_vec()), (10, outline.to_le_bytes().to_vec())];
        }
        let fill = if fill == 1 { fill3(1, &color3(1, [0, 100, 100, 0])) } else { fill3(0, &[]) };
        vec![(20, fill), (10, outline3(if outline == 2 { round } else { none }, color3(1, [0, 0, 0, 100]), &[]))]
    };
    let mut rect_args = style(1, 2);
    rect_args.push((30, v.rect(100.0, 50.0, 5.0)));
    let mut ellipse_args = style(1, 3);
    ellipse_args.push((30, [v.coord(40.0), v.coord(-40.0), v.angle(0.0), v.angle(0.0), v.word(0)].concat()));
    let mut curve_args = style(0, 2);
    curve_args.push((30, v.points(&[(0.0, 0.0, 0x08), (50.0, 0.0, 0x40), (50.0, 20.0, 0xC0), (20.0, 50.0, 0xC0), (0.0, 50.0, 0x88)])));
    let mut arc_args = style(0, 2);
    arc_args.push((30, [v.coord(30.0), v.coord(30.0), v.angle(0.0), v.angle(90.0), v.word(1)].concat()));
    let side = POLYGON_SIDE;
    let mut poly_args = style(1, 2);
    poly_args.push((30, v.points(&[(0.0, side, 0x0C), (side * 0.587, side * 0.809, 0x44), (side * 0.951, side * 0.309, 0x44)])));
    let mut poly = if version >= 1300 { vec![] } else { vec![0; 4] };
    poly.extend(5u32.to_le_bytes());
    poly.extend([0; 8]);
    if version >= 1300 {
        poly.extend([0; 4]);
    }
    poly.extend(0.05f64.to_le_bytes());
    poly.extend(0.05f64.to_le_bytes());
    poly.extend([v.coord(0.0), v.coord(0.0)].concat());
    poly_args.push((11000, poly));
    let art = layer(
        v,
        [0, 0, 0, 0x98],
        "Art",
        vec![
            object(v, 1, &rect_args, Some([1.0, 0.0, 0.0, 1.0, -100.0, 100.0])),
            object(v, 2, &ellipse_args, Some([1.0, 0.0, 0.0, 1.0, 50.0, -50.0])),
            object(v, 3, &curve_args, None),
            list(b"grp ", vec![chunk(b"flgs", vec![0, 0, 0, 0x10]), object(v, 0x14, &poly_args, None), object(v, 2, &arc_args, None)]),
            object(v, 4, &[(30, [v.coord(0.0), v.coord(0.0)].concat())], None),
        ],
    );
    let mut guide = vec![0; 12];
    guide.extend((90_000_000i32).to_le_bytes());
    guide.extend([v.coord(36.0), v.coord(0.0), v.coord(36.0), v.coord(100.0), v.coord(36.0), v.coord(0.0)].concat());
    let guides = layer(v, [0x0a, 0, 1, 0x98], "Guides", vec![object(v, 0x16, &[(16000, guide)], None)]);
    let desktop =
        layer(v, [0x08, 0, 1, 0x98], "Desktop", vec![object(v, 1, &[(30, v.rect(10.0, 10.0, 0.0))], Some([1.0, 0.0, 0.0, 1.0, 400.0, 0.0]))]);
    let pages = vec![page(true, vec![guides, desktop]), page(false, vec![art])];
    std::iter::once(doc).chain(pages).collect()
}

/// A drawing of `version` with one page of each kind of object this reader reads, a text object
/// it doesn't, a master page with a guideline and a desktop object; compressed (version 7 on)
/// when `compress`.
pub fn sample(version: u16, compress: bool) -> Vec<u8> {
    let content = sample_content(version);
    let mut body = vec![chunk(b"vrsn", version.to_le_bytes().to_vec())];
    if compress {
        body.push(cmpr(&content));
    } else {
        body.extend(content);
    }
    riff(version, &body)
}

/// The sample of `version` as the program of that version saves it: a RIFF file before 14, then
/// a package (R25, R26); `compress` compresses the RIFF stream before 16 and deflates the package
/// members.
pub fn sample_file(version: u16, compress: bool) -> Vec<u8> {
    match version {
        ..1400 => sample(version, compress),
        1400..1600 => package14(sample(version, compress), compress),
        _ => {
            // The full version chunk comes first from 16; the reader doesn't use it.
            let fver: Vec<u8> = [u32::from(version), 1, 0, u32::from(version / 100)].iter().flat_map(|v| v.to_le_bytes()).collect();
            let mut body = vec![chunk(b"fver", fver), chunk(b"vrsn", version.to_le_bytes().to_vec())];
            body.extend(sample_content(version));
            package16(version, &body, compress, &[])
        }
    }
}

/// One member of a synthetic ZIP archive.
#[derive(Clone, Debug)]
pub struct Member {
    pub name: String,
    pub data: Vec<u8>,
    pub deflate: bool,
}

fn deflate(data: &[u8]) -> Vec<u8> {
    let mut e = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
    let _ = e.write_all(data);
    e.finish().unwrap_or_default()
}

/// A ZIP archive of `members`, stored or deflated, with `comment` after the end record (R24).
pub fn zip(members: &[Member], comment: &[u8]) -> Vec<u8> {
    let (mut out, mut dir) = (Vec::new(), Vec::new());
    for m in members {
        let packed = if m.deflate { deflate(&m.data) } else { m.data.clone() };
        let method: u16 = if m.deflate { 8 } else { 0 };
        // Version needed, flags, method, time, date, CRC-32, packed and full sizes, name length.
        let mut common = Vec::new();
        common.extend(20u16.to_le_bytes());
        common.extend(0u16.to_le_bytes());
        common.extend(method.to_le_bytes());
        common.extend([0; 4]);
        common.extend(crc32fast::hash(&m.data).to_le_bytes());
        common.extend((packed.len() as u32).to_le_bytes());
        common.extend((m.data.len() as u32).to_le_bytes());
        common.extend((m.name.len() as u16).to_le_bytes());
        let at = out.len() as u32;
        out.extend(0x0403_4b50u32.to_le_bytes());
        out.extend(&common);
        out.extend(0u16.to_le_bytes());
        out.extend(m.name.as_bytes());
        out.extend(&packed);
        dir.extend(0x0201_4b50u32.to_le_bytes());
        dir.extend(20u16.to_le_bytes());
        dir.extend(&common);
        // Extra and comment lengths, disk, internal and external attributes, header offset.
        dir.extend([0; 12]);
        dir.extend(at.to_le_bytes());
        dir.extend(m.name.as_bytes());
    }
    let dir_at = out.len() as u32;
    let n = members.len() as u16;
    out.extend(&dir);
    out.extend(0x0605_4b50u32.to_le_bytes());
    out.extend([0; 4]);
    out.extend(n.to_le_bytes());
    out.extend(n.to_le_bytes());
    out.extend((dir.len() as u32).to_le_bytes());
    out.extend(dir_at.to_le_bytes());
    out.extend((comment.len() as u16).to_le_bytes());
    out.extend(comment);
    out
}

/// A version 14 or 15 package: the RIFF file `riff` as `content/riffData.cdr` (R25).
pub fn package14(riff: Vec<u8>, deflate: bool) -> Vec<u8> {
    zip(&[Member { name: "content/riffData.cdr".into(), data: riff, deflate }], b"")
}

/// A version 16+ package of a drawing of `version` holding `children` (R26, R27): the chunk tree
/// in `content/root.dat`, each chunk body of more than 8 bytes moved to a data stream (the
/// document's and other chunks' to `data1.dat`, each page's to `page<n>.dat`, each other list's
/// to a stream named after its form type) behind a redirect record, and shorter bodies kept in
/// theirs. Lists stay in the tree. `extra` members are added to the archive.
pub fn package16(version: u16, children: &[Node], deflate: bool, extra: &[Member]) -> Vec<u8> {
    package16_with(version, children, deflate, extra, &[])
}

/// [`package16`], moving the lists whose form types are in `moved_lists` whole into their stream.
pub fn package16_with(version: u16, children: &[Node], deflate: bool, extra: &[Member], moved_lists: &[[u8; 4]]) -> Vec<u8> {
    let mut members = package16_members(version, children, deflate, moved_lists);
    members.extend(extra.iter().cloned());
    zip(&members, b"")
}

/// The members of a [`package16_with`] archive.
pub fn package16_members(version: u16, children: &[Node], deflate: bool, moved_lists: &[[u8; 4]]) -> Vec<Member> {
    let mut streams: Vec<(String, Vec<u8>)> = vec![("data1.dat".into(), Vec::new())];
    let mut body = form(version).to_vec();
    for c in children {
        let s = match c {
            Node::List(f, _) if *f != *b"doc " => {
                let name = if *f == *b"page" { format!("page{}", streams.len()) } else { String::from_utf8_lossy(f).trim().to_string() };
                streams.push((format!("{name}.dat"), Vec::new()));
                streams.len() - 1
            }
            _ => 0,
        };
        body.extend(redirected(c, s, &mut streams, moved_lists));
    }
    let mut root = b"RIFF".to_vec();
    root.extend((body.len() as u32).to_le_bytes());
    root.extend(body);
    let list: Vec<String> = streams.iter().map(|(n, _)| n.clone()).collect();
    let mut members = vec![
        Member { name: "content/root.dat".into(), data: root, deflate },
        Member { name: "content/dataFileList.dat".into(), data: list.join("\n").into_bytes(), deflate },
    ];
    members.extend(streams.into_iter().map(|(n, data)| Member { name: format!("content/data/{n}"), data, deflate }));
    members
}

/// `n` laid out for `root.dat`, its bodies moved into stream `s`.
fn redirected(n: &Node, s: usize, streams: &mut [(String, Vec<u8>)], moved_lists: &[[u8; 4]]) -> Vec<u8> {
    let mut out = Vec::new();
    match n {
        Node::List(f, kids) if !moved_lists.contains(f) => {
            let mut body = f.to_vec();
            body.extend(kids.iter().flat_map(|k| redirected(k, s, streams, moved_lists)));
            put(&mut out, b"LIST", body.len() as u32, &body, body.len());
        }
        _ => {
            // A moved list's children are laid out in the stream by the same rule.
            let (id, data) = match n {
                Node::Chunk(id, data) => (*id, data.clone()),
                Node::List(f, kids) => {
                    let mut body = f.to_vec();
                    for k in kids {
                        body.extend(redirected(k, s, streams, moved_lists));
                    }
                    (*b"LIST", body)
                }
            };
            let mut rec = Vec::new();
            if data.len() <= 8 && id != *b"LIST" {
                rec.extend(u32::MAX.to_le_bytes());
                rec.extend((data.len() as u32).to_le_bytes());
                rec.extend(&data);
                rec.resize(16, 0);
            } else {
                let Some((_, stream)) = streams.get_mut(s) else { return out };
                rec.extend((s as u32).to_le_bytes());
                rec.extend((data.len() as u32).to_le_bytes());
                rec.extend((stream.len() as u32).to_le_bytes());
                rec.extend(0u32.to_le_bytes());
                stream.extend(&data);
            }
            put(&mut out, &id, 16, &rec, 16);
        }
    }
    out
}

/// The radius the base curve of a polygon lies on, 1000 mm, in points (R18).
pub const POLYGON_SIDE: f64 = 1000.0 / 25.4 * 72.0;

/// The shape of a drawn object in a file of version 1 or 2 (R43, R44), sizes in points.
#[derive(Clone, Debug)]
pub enum WlShape {
    Rect {
        w: f64,
        h: f64,
        radius: f64,
    },
    /// The two angle fields are written as given.
    Ellipse {
        w: f64,
        h: f64,
        angles: (i16, i16),
    },
    /// Points with their type bytes, as [`V::points`] takes them.
    Curve(Vec<(f64, f64, u8)>),
    Text,
}

/// A drawn object's record of a version 1 file (R42, R43, R44): `shape` placed by `matrix`
/// `[a, b, c, d, e, f]` with offsets in points, a uniform fill of `fill` and an outline of a width
/// in points and a color. The record's point holds the matrix's x offset and its `ty` the y
/// offset. [`wl_file`] moves the fill and the shape to their places in a version 2 file.
pub fn wl_object(shape: WlShape, matrix: [f64; 6], fill: Option<[u8; 5]>, outline: Option<(f64, [u8; 5])>) -> Vec<u8> {
    let v = V(300);
    let (kind, body): (u8, Vec<u8>) = match shape {
        WlShape::Rect { w, h, radius } => (0, [v.coord(w), v.coord(h), v.coord(radius), vec![0; 2]].concat()),
        WlShape::Ellipse { w, h, angles } => (1, [v.coord(w), v.coord(h), angles.0.to_le_bytes().to_vec(), angles.1.to_le_bytes().to_vec()].concat()),
        WlShape::Curve(points) => (2, v.points(&points)),
        WlShape::Text => (3, vec![0; 30]),
    };
    let mut r = vec![0u8; (114 + body.len()).max(118)];
    r[..4].copy_from_slice(&WL_RECORD_START);
    r[26] = 0x40;
    r[27] = 0x20;
    let size = (r.len() - 32) as u16;
    r[28..30].copy_from_slice(&size.to_le_bytes());
    let [a, b, c, d, e, f] = matrix;
    r[32..34].copy_from_slice(&v.coord(e));
    let fixed = |x: f64| ((x * 65536.0).round() as i32).to_le_bytes();
    // a, b, tx, c, d, ty: x' = a x + b y + tx, y' = c x + d y + ty.
    for (k, x) in [a, c, 0.0, b, d, f / 72.0 * 1000.0].into_iter().enumerate() {
        r[36 + 4 * k..40 + 4 * k].copy_from_slice(&fixed(x));
    }
    r[60] = kind;
    if let Some((width, color)) = outline {
        r[67] = 1;
        r[68..70].copy_from_slice(&v.coord(width));
        r[74..79].copy_from_slice(&color);
    }
    if let Some(color) = fill {
        r[94] = 1;
        r[95..100].copy_from_slice(&color);
    }
    r[114..114 + body.len()].copy_from_slice(&body);
    r
}

/// The u32 that starts every record of a version 1 or 2 file (R42).
const WL_RECORD_START: [u8; 4] = 24u32.to_le_bytes();

/// A group or layer record of a version 1 or 2 file (R42): 28 bytes.
pub fn wl_group() -> Vec<u8> {
    let mut r = vec![0u8; 28];
    r[..4].copy_from_slice(&WL_RECORD_START);
    r[26] = 1;
    r[27] = 0x60;
    r
}

/// A drawn object's record of a version 1 file laid out as in a version 2 file (R43): 11 more bytes
/// before the fill and 11 more before the shape. Group records stay as they are.
fn wl_v2_record(rec: &[u8]) -> Vec<u8> {
    if rec.get(26).is_none_or(|flags| flags & 0x40 == 0) || rec.len() < 114 {
        return rec.to_vec();
    }
    let mut out = [&rec[..94], &[0; 11], &rec[94..114], &[0; 11], &rec[114..]].concat();
    let size = (out.len() - 32) as u16;
    out[28..30].copy_from_slice(&size.to_le_bytes());
    out
}

/// A file of version 1 or 2 (R40, R41): the header with `marker` (`e` for version 1, `l` or `m`
/// for version 2), a page of `page` points, the `records` and an empty font table. A version 1
/// file's settings start after the header; a version 2 file's start further on, where a picture
/// lies in the files of that version, and its drawn objects are laid out as [`wl_v2_record`] says.
pub fn wl_file(marker: u8, page: (f64, f64), records: &[Vec<u8>]) -> Vec<u8> {
    let v = V(300);
    let settings = if marker == b'e' { 28u32 } else { 40 };
    let start = 64u32;
    let mut out = vec![0u8; start as usize];
    out[..4].copy_from_slice(&[b'W', b'L', marker, 0]);
    let (w, h) = page;
    let page_box = settings as usize + 2;
    out[page_box..page_box + 8].copy_from_slice(&[v.coord(-w / 2.0), v.coord(h / 2.0), v.coord(w / 2.0), v.coord(-h / 2.0)].concat());
    for r in records {
        if marker == b'e' {
            out.extend(r);
        } else {
            out.extend(wl_v2_record(r));
        }
    }
    let end = out.len() as u32;
    out.extend([0; 2]);
    let trailer = out.len() as u32;
    for (k, offset) in [settings, start, end, trailer, trailer - 2].into_iter().enumerate() {
        out[4 + 4 * k..8 + 4 * k].copy_from_slice(&offset.to_le_bytes());
    }
    out
}

/// A drawing of version 1 or 2 with a group record and one of each kind of object: a rounded
/// rectangle with a fill and an outline, an ellipse, a closed curve and a text object, front to
/// back.
pub fn wl_sample(marker: u8) -> Vec<u8> {
    let (red, black) = (color3(1, [0, 100, 100, 0]), color3(1, [0, 0, 0, 100]));
    let curve = vec![(0.0, 0.0, 0x08), (50.0, 0.0, 0x40), (50.0, 20.0, 0xC0), (20.0, 50.0, 0xC0), (0.0, 50.0, 0x88)];
    let records = [
        wl_object(WlShape::Rect { w: 100.0, h: 50.0, radius: 5.0 }, [1.0, 0.0, 0.0, 1.0, -100.0, 100.0], Some(red), Some((2.0, black))),
        wl_group(),
        wl_object(WlShape::Ellipse { w: 40.0, h: -40.0, angles: (0, 0) }, [1.0, 0.0, 0.0, 1.0, 50.0, -50.0], Some(red), None),
        wl_object(WlShape::Curve(curve), [2.0, 0.0, 0.0, 2.0, 0.0, 0.0], None, Some((2.0, black))),
        wl_object(WlShape::Text, [1.0, 0.0, 0.0, 1.0, 0.0, 0.0], Some(black), None),
    ];
    wl_file(marker, (612.0, 792.0), &records)
}
