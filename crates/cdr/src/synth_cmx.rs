//! Synthetic `.cmx` files for tests and fuzz seeds, built from the layouts in `PROVENANCE.md` (R53
//! to R63). Not a `.cmx` writer: nothing checks that other programs open these files.

/// A chunk: its id, a u32 size, the body and a pad byte after an odd body (R53).
pub fn chunk(id: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut out = id.to_vec();
    out.extend((body.len() as u32).to_le_bytes());
    out.extend(body);
    if body.len() % 2 == 1 {
        out.push(0);
    }
    out
}

/// A `LIST` chunk of form `form`.
pub fn list(form: &[u8; 4], children: &[Vec<u8>]) -> Vec<u8> {
    let mut body = form.to_vec();
    body.extend(children.concat());
    chunk(b"LIST", &body)
}

/// The RIFF form `CMX1` around `children`.
pub fn riff(children: &[Vec<u8>]) -> Vec<u8> {
    let mut body = b"CMX1".to_vec();
    body.extend(children.concat());
    chunk(b"RIFF", &body)
}

/// The header (R54): `coord` is the coordinate size digit (`b'4'` or `b'2'`) and `meters` the size
/// of a coordinate unit. The header string and the system name, which the reader doesn't read,
/// are left as zeros.
pub fn cont(coord: u8, meters: f64) -> Vec<u8> {
    let mut c = vec![0u8; 180];
    c[48] = b'2';
    c[52] = coord;
    c[54] = b'2';
    c[64..72].copy_from_slice(&meters.to_le_bytes());
    chunk(b"cont", &c)
}

/// A field: its tag, a u16 length counting the 3 header bytes, then `data` (R55).
pub fn field(tag: u8, data: &[u8]) -> Vec<u8> {
    let mut f = vec![tag];
    f.extend(((data.len() + 3) as u16).to_le_bytes());
    f.extend(data);
    f
}

/// A field list: the fields, then 0xff (R55).
pub fn fields(fields: &[Vec<u8>]) -> Vec<u8> {
    let mut l = fields.concat();
    l.push(0xff);
    l
}

/// A table chunk: a u16 count, then the entries (field lists) (R55).
pub fn table(id: &[u8; 4], entries: &[Vec<u8>]) -> Vec<u8> {
    let mut d = (entries.len() as u16).to_le_bytes().to_vec();
    d.extend(entries.concat());
    chunk(id, &d)
}

/// A color entry in the older form: the model and palette bytes in field 1, the values in field 2
/// (R56).
pub fn color(model: u8, values: &[u8]) -> Vec<u8> {
    fields(&[field(1, &[model, 5]), field(2, values)])
}

/// A color entry with a property list in field 10 whose color record (part 1) holds `model` and
/// the four value bytes `v` (R56).
pub fn color_record(model: u16, v: [u8; 4]) -> Vec<u8> {
    color_record_named(model, v, None)
}

/// A color entry as [`color_record`], with a name part (part 3: a u32 count and UTF-16 units)
/// after the color when `name` is given (R56).
pub fn color_record_named(model: u16, v: [u8; 4], name: Option<&str>) -> Vec<u8> {
    let mut part = vec![1u8];
    part.extend(12u32.to_le_bytes());
    part.extend(model.to_le_bytes());
    part.extend(5u16.to_le_bytes());
    part.extend([0; 4]);
    part.extend(v);
    if let Some(name) = name {
        let units: Vec<u16> = name.encode_utf16().collect();
        part.push(3);
        part.extend((4 + 2 * units.len() as u32).to_le_bytes());
        part.extend((units.len() as u32).to_le_bytes());
        part.extend(units.iter().flat_map(|u| u.to_le_bytes()));
    }
    let mut rec = (part.len() as u32).to_le_bytes().to_vec();
    rec.extend(part);
    fields(&[field(10, &rec)])
}

/// A pen entry (R57): the width in coordinate units, an aspect of 100 and no angle, with the
/// identity matrix.
pub fn pen(width: u32) -> Vec<u8> {
    let mut d = width.to_le_bytes().to_vec();
    d.extend(100u16.to_le_bytes());
    d.extend(0i32.to_le_bytes());
    d.extend(1u16.to_le_bytes());
    fields(&[field(1, &d)])
}

/// A line type entry (R57): its flags and the caps (low 4 bits) and join (high 4 bits).
pub fn line_type(flags: u8, caps_join: u8) -> Vec<u8> {
    fields(&[field(1, &[flags, caps_join])])
}

/// A dash pattern entry (R57): the lengths in outline widths.
pub fn dashes(lengths: &[u16]) -> Vec<u8> {
    let mut d = (lengths.len() as u16).to_le_bytes().to_vec();
    d.extend(lengths.iter().flat_map(|l| l.to_le_bytes()));
    fields(&[field(1, &d)])
}

/// An outline entry (R58): 1-based indexes of the line type, screen, color, arrowhead pair, pen and
/// dash pattern.
pub fn outline(parts: [u16; 6]) -> Vec<u8> {
    fields(&[field(1, &parts.iter().flat_map(|p| p.to_le_bytes()).collect::<Vec<u8>>())])
}

/// The arrowhead pairs chunk (R59): a u16 count, then start and end (u16 each).
pub fn arrowheads(pairs: &[(u16, u16)]) -> Vec<u8> {
    let mut d = (pairs.len() as u16).to_le_bytes().to_vec();
    for (s, e) in pairs {
        d.extend(s.to_le_bytes());
        d.extend(e.to_le_bytes());
    }
    chunk(b"rota", &d)
}

/// An instruction in the short form: a u16 size counting the whole instruction, the code, the
/// body (R60).
pub fn instr(code: i16, body: &[u8]) -> Vec<u8> {
    let mut i = ((body.len() + 4) as u16).to_le_bytes().to_vec();
    i.extend(code.to_le_bytes());
    i.extend(body);
    i
}

/// An instruction in the long form: 0xFFFF, a u32 size counting the whole instruction, the code,
/// the body (R60).
pub fn long_instr(code: i16, body: &[u8]) -> Vec<u8> {
    let mut i = 0xFFFFu16.to_le_bytes().to_vec();
    i.extend(((body.len() + 8) as u32).to_le_bytes());
    i.extend(code.to_le_bytes());
    i.extend(body);
    i
}

/// Begin a page (R61), with its width and height in coordinate units in field 4 when given.
pub fn begin_page(size: Option<(u32, u32)>) -> Vec<u8> {
    let mut f = vec![field(1, &[1, 0, 0, 0, 0, 0])];
    if let Some((w, h)) = size {
        let mut d = w.to_le_bytes().to_vec();
        d.extend(h.to_le_bytes());
        f.push(field(4, &d));
    }
    instr(9, &fields(&f))
}

/// Begin a page (R61) without a size of its own, with the drawing's left, top, right and bottom
/// edges in field 1.
pub fn begin_page_in_bounds(left: i32, top: i32, right: i32, bottom: i32) -> Vec<u8> {
    let mut d = vec![1, 0, 0, 0, 0, 0];
    for v in [left, top, right, bottom] {
        d.extend(v.to_le_bytes());
    }
    instr(9, &fields(&[field(1, &d)]))
}

/// Begin a layer (R62) named in 8-bit characters in field 1 from byte 12, and in UTF-16 in
/// field 4 when `wide` is given.
pub fn begin_layer(name: &str, wide: Option<&str>) -> Vec<u8> {
    let mut d = vec![1, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    d.extend((name.len() as u16).to_le_bytes());
    d.extend(name.bytes());
    let mut f = vec![field(1, &d)];
    if let Some(w) = wide {
        let units: Vec<u16> = w.encode_utf16().collect();
        let mut d = (units.len() as u32).to_le_bytes().to_vec();
        d.extend(units.iter().flat_map(|u| u.to_le_bytes()));
        f.push(field(4, &d));
    }
    instr(11, &fields(&f))
}

pub fn end_layer() -> Vec<u8> {
    instr(12, &[])
}

pub fn end_page() -> Vec<u8> {
    instr(10, &[])
}

pub fn begin_group() -> Vec<u8> {
    instr(13, &fields(&[field(1, &[0; 26])]))
}

pub fn end_group() -> Vec<u8> {
    instr(14, &[])
}

/// Rendering attributes (R63): a uniform fill of color `fill` and the outline `outline` (1-based
/// indexes; `None` leaves the part out of the mask), and `lens` sets the lens bit.
pub fn attrs(fill: Option<u16>, outline: Option<u16>, lens: bool) -> Vec<u8> {
    let mask = u8::from(fill.is_some()) | (u8::from(outline.is_some()) << 1) | (u8::from(lens) << 2);
    let mut a = vec![mask];
    if let Some(c) = fill {
        let mut v = 1u16.to_le_bytes().to_vec();
        let mut uniform = c.to_le_bytes().to_vec();
        uniform.extend(0u16.to_le_bytes());
        v.extend(fields(&[field(1, &uniform)]));
        a.extend(fields(&[field(1, &v)]));
    }
    if let Some(o) = outline {
        a.extend(fields(&[field(1, &o.to_le_bytes())]));
    }
    a
}

/// Rendering attributes with a fill of type `kind` and no settings of its own.
pub fn attrs_fill_type(kind: u16) -> Vec<u8> {
    let mut a = vec![1u8];
    a.extend(fields(&[field(1, &kind.to_le_bytes())]));
    a
}

/// A curve (R62): `attrs`, then a u16 count, the points (i32 x, y) and their type bytes.
pub fn curve(attrs: &[u8], points: &[(i32, i32)], types: &[u8]) -> Vec<u8> {
    let mut g = (points.len() as u16).to_le_bytes().to_vec();
    for (x, y) in points {
        g.extend(x.to_le_bytes());
        g.extend(y.to_le_bytes());
    }
    g.extend(types);
    instr(67, &fields(&[field(1, attrs), field(2, &g), field(3, &[0; 16])]))
}

/// A closed square of side `side` with its lower left corner at (`x`, `y`).
pub fn square(attrs: &[u8], x: i32, y: i32, side: i32) -> Vec<u8> {
    curve(attrs, &[(x, y), (x + side, y), (x + side, y + side), (x, y + side), (x, y)], &[0x0c, 0x44, 0x44, 0x44, 0x48])
}

/// A `page` chunk of `instructions`.
pub fn page(instructions: &[Vec<u8>]) -> Vec<u8> {
    chunk(b"page", &instructions.concat())
}

/// The tables of [`sample`]: colors red (RGB), black (CMYK in percent) and blue (a color record,
/// blue, green, red); pens of 0.5 mm and 1 mm; line types solid with round caps and joins, none,
/// and dashed behind the fill; one dash pattern; three outlines; one empty arrowhead pair.
pub fn tables() -> Vec<Vec<u8>> {
    vec![
        table(b"rclr", &[color(5, &[255, 0, 0]), color(2, &[0, 0, 0, 100]), color_record(5, [255, 0, 0, 0])]),
        table(b"rpen", &[pen(5_000), pen(10_000)]),
        table(b"rott", &[line_type(2, 0x11), line_type(1, 0), line_type(2 | 4 | 8, 0x02)]),
        table(b"rdot", &[dashes(&[3, 1])]),
        table(b"rotl", &[outline([1, 1, 2, 1, 1, 1]), outline([2, 1, 2, 1, 1, 1]), outline([3, 1, 3, 1, 2, 1])]),
        arrowheads(&[(0, 0)]),
    ]
}

/// One page of 210 x 297 mm with two layers: "Art" holds a red square outlined in black, then a
/// group of a square with no outline and a blue one with the dashed outline; "Top" holds a black
/// square drawn last. Units are 0.1 µm.
pub fn sample() -> Vec<u8> {
    let p = page(&[
        begin_page(Some((2_100_000, 2_970_000))),
        begin_layer("Art", None),
        square(&attrs(Some(1), Some(1), false), -500_000, -500_000, 1_000_000),
        begin_group(),
        square(&attrs(Some(2), Some(2), false), 0, 0, 100_000),
        square(&attrs(Some(3), Some(3), false), 200_000, 0, 100_000),
        end_group(),
        end_layer(),
        begin_layer("Top", Some("Top")),
        square(&attrs(Some(2), None, false), 0, 0, 10_000),
        end_layer(),
        end_page(),
    ]);
    let mut children = vec![cont(b'4', 1e-7), p];
    children.extend(tables());
    riff(&children)
}
