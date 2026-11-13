//! Low-level raster and spot-plate PDF serialization.
//!
//! Callers map their page and color models to these neutral records. This module owns PDF object
//! layout, image streams, ICC/Separation resources, page labels, marks, xref and trailer syntax.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::io::Write as _;

/// An 8-bit interleaved image placed on one page: gray, RGB or CMYK.
#[derive(Clone, Debug, Default)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub channels: usize,
    pub data: Vec<u8>,
    pub icc: Option<Vec<u8>>,
}

/// One spot plate with 8-bit ink density and an alternate CMYK preview color.
#[derive(Clone, Debug, Default)]
pub struct SpotPlate {
    pub name: String,
    pub cmyk: [f32; 4],
    /// CIE L*a*b* ink color for a caller's metadata packet.
    pub lab: [f32; 3],
    /// Encoded sRGB ink color for a caller's metadata packet.
    pub srgb: [f32; 3],
    /// Spot ink solidity in the inclusive range 0..=1.
    pub solidity: f32,
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

/// Crop and registration marks around a page's image rectangle.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Marks {
    pub corner_crop: bool,
    pub center_crop: bool,
    pub registration: bool,
}

/// A neutral PDF page and its raster/spot resources.
#[derive(Clone, Debug, Default)]
pub struct Page {
    pub paper: (f64, f64),
    pub image: Option<Image>,
    pub rect: (f64, f64, f64, f64),
    pub marks: Marks,
    pub description: Option<String>,
    pub label: Option<String>,
    pub page_label: Option<String>,
    pub plates: Vec<SpotPlate>,
}

/// A one-byte Unicode map for invisible, selectable text layered over vector outlines.
#[derive(Default)]
pub struct SelectionFont {
    chars: BTreeSet<char>,
}

impl SelectionFont {
    /// Collect distinct characters until the one-byte code space is exceeded.
    pub fn new<'a>(texts: impl Iterator<Item = &'a str>) -> Self {
        let mut chars = BTreeSet::new();
        'texts: for text in texts {
            for ch in text.chars() {
                chars.insert(ch);
                if chars.len() > 255 {
                    break 'texts;
                }
            }
        }
        Self { chars }
    }
    /// Encode text as hex character codes in this font's Unicode map.
    pub fn encode(&self, text: &str) -> Option<String> {
        if self.chars.len() > 255 {
            return None;
        }
        let mut encoded = String::new();
        for ch in text.chars() {
            let code = self.chars.iter().position(|c| *c == ch)?.checked_add(1)?;
            write!(encoded, "{code:02X}").ok()?;
        }
        Some(encoded)
    }
    fn objects(&self, base: usize) -> Vec<Vec<u8>> {
        let count = self.chars.len().min(255);
        let mut differences = String::new();
        let mut procs = String::new();
        let mut cmap = String::from(
            "/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n/CMapName /PhotoCraftUnicode def\n/CMapType 2 def\n1 begincodespacerange\n<00> <FF>\nendcodespacerange\n",
        );
        let chars: Vec<_> = self.chars.iter().take(count).collect();
        for (block, chunk) in chars.chunks(100).enumerate() {
            let _ = writeln!(cmap, "{} beginbfchar", chunk.len());
            for (i, ch) in chunk.iter().enumerate() {
                let code = block * 100 + i + 1;
                let mut buf = [0; 2];
                let unicode = ch.encode_utf16(&mut buf).iter().map(|u| format!("{u:04X}")).collect::<String>();
                let _ = writeln!(cmap, "<{code:02X}> <{unicode}>");
            }
            cmap.push_str("endbfchar\n");
        }
        cmap.push_str("endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n");
        for code in 1..=count {
            let _ = write!(differences, " /c{code}");
            let _ = write!(procs, " /c{code} {} 0 R", base + 1);
        }
        let widths = "1000 ".repeat(count.max(1));
        let font = format!(
            "<< /Type /Font /Subtype /Type3 /Name /FSelect /FontBBox [0 -250 1000 1000] /FontMatrix [0.001 0 0 0.001 0 0] /CharProcs <<{procs} >> /Encoding << /Type /Encoding /Differences [1{differences}] >> /FirstChar 1 /LastChar {} /Widths [{widths}] /Resources << >> /ToUnicode {} 0 R >>",
            count.max(1),
            base + 2
        );
        let stream = |data: &str| format!("<< /Length {} >>\nstream\n{data}endstream", data.len()).into_bytes();
        vec![font.into_bytes(), stream("1000 0 0 -250 1000 1000 d1\n"), stream(&cmap)]
    }
}

fn pdf_string(s: &str) -> String {
    let mut o = String::from("(");
    for c in s.chars() {
        match c {
            '(' | ')' | '\\' => {
                o.push('\\');
                o.push(c);
            }
            c if c.is_ascii() && !c.is_ascii_control() => o.push(c),
            _ => o.push('?'),
        }
    }
    o.push(')');
    o
}

/// A PDF text string with UTF-16BE hex (the Unicode-capable form, byte-order mark included).
fn pdf_unicode_string(s: &str) -> String {
    let mut o = String::from("<FEFF");
    for u in s.encode_utf16() {
        o.push_str(&format!("{u:04X}"));
    }
    o.push('>');
    o
}

/// `s` as a PDF name object (ISO 32000-1 §7.3.5): `/` then the UTF-8 bytes, every byte outside
/// the regular printable ASCII range, `#` and the delimiters as `#xx`. An empty name would be
/// the bare `/`, which some readers reject, so it becomes `/Spot`.
fn pdf_name(s: &str) -> String {
    let mut o = String::from("/");
    for b in s.bytes() {
        let regular = (0x21..=0x7e).contains(&b) && !matches!(b, b'#' | b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%');
        if regular {
            o.push(char::from(b));
        } else {
            o.push_str(&format!("#{b:02X}"));
        }
    }
    if o.len() == 1 {
        o.push_str("Spot");
    }
    o
}

fn deflate(data: &[u8]) -> Vec<u8> {
    // Fast: print PDFs are transient and large. Level 3, not 1: zlib-rs level 1 writes static
    // Huffman blocks only (13-30% bigger on photos); level 3 is the lowest that is never bigger
    // than miniz's level 1 was, at about twice its time.
    let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::new(3));
    let _ = e.write_all(data);
    e.finish().unwrap_or_default()
}

/// Vector marks around `(x, y, w, h)`.
fn marks_ops(m: Marks, (x, y, w, h): (f64, f64, f64, f64)) -> String {
    let mut s = String::from("q 0 G 0.3 w\n");
    let (gap, len) = (6.0, 18.0);
    let line = |s: &mut String, x0: f64, y0: f64, x1: f64, y1: f64| s.push_str(&format!("{x0:.2} {y0:.2} m {x1:.2} {y1:.2} l S\n"));
    if m.corner_crop {
        for (cx, cy, dx, dy) in [(x, y, -1.0, -1.0), (x + w, y, 1.0, -1.0), (x, y + h, -1.0, 1.0), (x + w, y + h, 1.0, 1.0)] {
            line(&mut s, cx + dx * gap, cy, cx + dx * (gap + len), cy);
            line(&mut s, cx, cy + dy * gap, cx, cy + dy * (gap + len));
        }
    }
    if m.center_crop {
        let (mx, my) = (x + w / 2.0, y + h / 2.0);
        line(&mut s, mx, y - gap, mx, y - gap - len);
        line(&mut s, mx, y + h + gap, mx, y + h + gap + len);
        line(&mut s, x - gap, my, x - gap - len, my);
        line(&mut s, x + w + gap, my, x + w + gap + len, my);
    }
    if m.registration {
        let r = 5.0;
        let off = gap + len / 2.0;
        for (cx, cy) in [(x + w / 2.0, y - off), (x + w / 2.0, y + h + off), (x - off, y + h / 2.0), (x + w + off, y + h / 2.0)] {
            // Circle (four Béziers) plus crosshair.
            let k = 0.5523 * r;
            s.push_str(&format!(
                "{:.2} {cy:.2} m {:.2} {:.2} {:.2} {:.2} {cx:.2} {:.2} c {:.2} {:.2} {:.2} {:.2} {:.2} {cy:.2} c {:.2} {:.2} {:.2} {:.2} {cx:.2} {:.2} c {:.2} {:.2} {:.2} {:.2} {:.2} {cy:.2} c S\n",
                cx + r, cx + r, cy + k, cx + k, cy + r, cy + r, cx - k, cy + r, cx - r, cy + k, cx - r, cx - r, cy - k, cx - k, cy - r, cy - r, cx + k, cy - r, cx + r, cy - k, cx + r
            ));
            line(&mut s, cx - r * 1.6, cy, cx + r * 1.6, cy);
            line(&mut s, cx, cy - r * 1.6, cx, cy + r * 1.6);
        }
    }
    s.push_str("Q\n");
    s
}

/// An image XObject dictionary and its Flate-compressed samples, as one object body.
fn image_object(width: u32, height: u32, colour_space: &str, data: &[u8]) -> Vec<u8> {
    let z = deflate(data);
    let mut body = format!(
        "<< /Type /XObject /Subtype /Image /Width {width} /Height {height} /ColorSpace {colour_space} /BitsPerComponent 8 /Filter /FlateDecode /Length {} >>\nstream\n",
        z.len()
    )
    .into_bytes();
    body.extend_from_slice(&z);
    body.extend_from_slice(b"\nendstream");
    body
}

/// A PDF with one page per entry; `vectors[i]` (when present) is drawn over page `i`, after its
/// image and before its plates and marks (the top layers kept as paths and selectable text), and
/// the selection font their text refers to is `/FSelect`. `xmp` is the packet of the catalog's
/// `/Metadata` stream, if any.
///
/// Objects: 1 catalog, 2 pages, then per page its page, contents, image and one XObject per
/// plate; then the shared font, ICC profile, overprint ExtGState and XMP metadata stream, each
/// only when present, the selection font's three objects, and last the page-label tree.
pub fn write_pages(pages: &[Page], vectors: &[String], selection_font: &SelectionFont, xmp: Option<&str>) -> Vec<u8> {
    if pages.is_empty() {
        return Vec::new();
    }
    let mut out: Vec<u8> = b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n".to_vec();
    let mut offsets: Vec<usize> = Vec::new();
    let mut obj = |out: &mut Vec<u8>, body: &[u8]| {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", offsets.len()).as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    };
    let n = pages.len();
    // A label tree is written only for a non-empty name: an all-empty tree would be `/Nums []`,
    // and filtering the first empty name away would start the tree at page index 1, not 0.
    let labels_on = pages.iter().any(|p| p.page_label.as_deref().is_some_and(|label| !label.is_empty()));
    // Object numbers, handed out in the order the objects are written.
    let mut next = 3usize;
    let mut take = |present: bool| {
        let o = present.then_some(next);
        next += usize::from(present);
        o
    };
    struct PageObjects {
        page: usize,
        contents: usize,
        image: Option<usize>,
        plates: Vec<usize>,
    }
    // Field initialisers run in this order.
    let page_objects: Vec<PageObjects> = pages
        .iter()
        .map(|p| PageObjects {
            page: take(true).unwrap_or_default(),
            contents: take(true).unwrap_or_default(),
            image: take(p.image.is_some()),
            plates: p.plates.iter().filter_map(|_| take(true)).collect(),
        })
        .collect();
    let font_obj = take(true).unwrap_or_default();
    // The profile is the document's, shared by every page's image.
    let icc = pages.iter().find_map(|p| p.image.as_ref().and_then(|i| i.icc.as_ref().map(|icc| (icc, i.channels))));
    let icc_obj = take(icc.is_some());
    let gs_obj = take(pages.iter().any(|p| !p.plates.is_empty()));
    let xmp_obj = take(xmp.is_some());
    let selection_base = next;
    let selection_objects = selection_font.objects(selection_base);
    let labels_obj = labels_on.then_some(selection_base + selection_objects.len());

    let metadata = xmp_obj.map(|o| format!(" /Metadata {o} 0 R")).unwrap_or_default();
    let page_labels = labels_obj.map(|o| format!(" /PageLabels {o} 0 R")).unwrap_or_default();
    obj(&mut out, format!("<< /Type /Catalog /Pages 2 0 R{metadata}{page_labels} >>").as_bytes());
    let kids = page_objects.iter().map(|p| format!("{} 0 R", p.page)).collect::<Vec<_>>().join(" ");
    obj(&mut out, format!("<< /Type /Pages /Kids [{kids}] /Count {n} >>").as_bytes());
    for (i, (page, objects)) in pages.iter().zip(&page_objects).enumerate() {
        let (pw, ph) = page.paper;
        let (x, y, w, h) = page.rect;
        let mut xobjects = String::new();
        if let Some(o) = objects.image {
            xobjects.push_str(&format!("/Im0 {o} 0 R "));
        }
        for (k, o) in objects.plates.iter().enumerate() {
            xobjects.push_str(&format!("/Sp{k} {o} 0 R "));
        }
        let ext_gstate = match gs_obj {
            Some(o) if !page.plates.is_empty() => format!("/ExtGState << /GS0 {o} 0 R >> "),
            _ => String::new(),
        };
        obj(
            &mut out,
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {pw:.2} {ph:.2}] /Resources << /XObject << {xobjects}>> {ext_gstate}/Font << /F1 {font_obj} 0 R /FSelect {selection_base} 0 R >> >> /Contents {} 0 R >>",
                objects.contents
            )
            .as_bytes(),
        );
        // An explicit white page, so every viewer (and rasteriser) shows paper, not transparency.
        let mut content = format!("q 1 g 0 0 {pw:.2} {ph:.2} re f Q\n");
        if objects.image.is_some() {
            content.push_str(&format!("q {w:.3} 0 0 {h:.3} {x:.3} {y:.3} cm /Im0 Do Q\n"));
        }
        if let Some(v) = vectors.get(i) {
            content.push_str(v);
        }
        // Each plate overprints what is already on the page: with `/OP true` a Separation paints
        // its own colorant only, leaving the others (and the composite) as they are.
        for k in 0..page.plates.len() {
            content.push_str(&format!("q /GS0 gs {w:.3} 0 0 {h:.3} {x:.3} {y:.3} cm /Sp{k} Do Q\n"));
        }
        content.push_str(&marks_ops(page.marks, page.rect));
        let any_marks = page.marks.corner_crop || page.marks.center_crop || page.marks.registration;
        let pad = if any_marks { 30.0 } else { 6.0 };
        if let Some(l) = page.label.as_deref().filter(|l| !l.is_empty()) {
            content.push_str(&format!("BT /F1 8 Tf 0 g {:.2} {:.2} Td {} Tj ET\n", x, y + h + pad, pdf_string(l)));
        }
        if let Some(d) = page.description.as_deref().filter(|d| !d.is_empty()) {
            content.push_str(&format!("BT /F1 8 Tf 0 g {:.2} {:.2} Td {} Tj ET\n", x, y - pad - 8.0, pdf_string(d)));
        }
        let c = deflate(content.as_bytes());
        let mut body = format!("<< /Length {} /Filter /FlateDecode >>\nstream\n", c.len()).into_bytes();
        body.extend_from_slice(&c);
        body.extend_from_slice(b"\nendstream");
        obj(&mut out, &body);
        if let Some(img) = &page.image {
            let device = match img.channels {
                1 => "/DeviceGray",
                4 => "/DeviceCMYK",
                _ => "/DeviceRGB",
            };
            let cs = match icc_obj {
                Some(o) if img.icc.is_some() => format!("[/ICCBased {o} 0 R]"),
                _ => device.to_string(),
            };
            obj(&mut out, &image_object(img.width, img.height, &cs, &img.data));
        }
        for p in &page.plates {
            // A tint transform from the colorant's density to its alternate: 0 → paper, 1 → `cmyk`
            // (an exponential interpolation function with N = 1 is linear).
            let cmyk = p.cmyk.map(|v| format!("{:.4}", v.clamp(0.0, 1.0))).join(" ");
            let cs = format!("[/Separation {} /DeviceCMYK << /FunctionType 2 /Domain [0 1] /C0 [0 0 0 0] /C1 [{cmyk}] /N 1 >>]", pdf_name(&p.name));
            // No mask on the plate: where it has no ink it paints "0" of its own colorant, which a
            // separating RIP reads as no ink and overprint keeps the other plates under. (A stencil
            // or colour-key mask would let plain viewers show every plate, but Illustrator reports
            // an explicit image mask as an unknown imaging construct, and Quartz draws nothing for a
            // colour-key-masked Separation image.)
            obj(&mut out, &image_object(p.width, p.height, &cs, &p.data));
        }
    }
    obj(&mut out, b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>");
    if let Some((icc, channels)) = icc {
        let device = match channels {
            1 => "/DeviceGray",
            4 => "/DeviceCMYK",
            _ => "/DeviceRGB",
        };
        let z = deflate(icc);
        let mut body = format!("<< /N {channels} /Alternate {device} /Filter /FlateDecode /Length {} >>\nstream\n", z.len()).into_bytes();
        body.extend_from_slice(&z);
        body.extend_from_slice(b"\nendstream");
        obj(&mut out, &body);
    }
    if gs_obj.is_some() {
        obj(&mut out, b"<< /Type /ExtGState /OP true /op true /OPM 1 >>");
    }
    if let Some(xmp) = xmp {
        // Unfiltered, as the XMP specification asks of a packet in a PDF, so a reader can find it
        // by scanning for the packet header.
        let mut body = format!("<< /Type /Metadata /Subtype /XML /Length {} >>\nstream\n", xmp.len()).into_bytes();
        body.extend_from_slice(xmp.as_bytes());
        body.extend_from_slice(b"\nendstream");
        obj(&mut out, &body);
    }
    for body in &selection_objects {
        obj(&mut out, body);
    }
    if labels_obj.is_some() {
        // Every page gets an entry: a number-tree range runs until the next key, so omitting an
        // empty name would make that page inherit the previous page's label. An empty
        // dictionary gives the page the default (decimal) label instead.
        let nums = pages
            .iter()
            .enumerate()
            .map(|(i, p)| match p.page_label.as_deref().filter(|label| !label.is_empty()) {
                Some(label) => format!("{i} << /P {} >>", pdf_unicode_string(label)),
                None => format!("{i} << >>"),
            })
            .collect::<Vec<_>>()
            .join(" ");
        obj(&mut out, format!("<< /Nums [{nums}] >>").as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len() + 1).as_bytes());
    for o in &offsets {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", offsets.len() + 1).as_bytes());
    out
}
