//! PdfCraft hayro-interpret patch (119), storytold/pdfcraft#7492: a shading painted with `sh` in
//! a tiling pattern's cell fills the cell's `/BBox` (ISO 32000-2 8.7.3.1, 8.7.4.2), wherever the
//! cell lies in pattern space.
//! PDFBOX-5852's `minimal.pdf` draws a Type 4 mesh in a cell under a pattern matrix that flips y,
//! so the mesh lies at negative pattern coordinates, and it was not drawn at all: the cell's
//! drawing area was the device rectangle of the page, read as pattern coordinates. Adapted from
//! PdfCraft's `tiling_cell_shading` renderer test.

use crate::{InterpreterSettings, Pdf, RenderCache, RenderSettings, render};
use vello_cpu::Pixmap;
use vello_cpu::color::palette::css::WHITE;

/// A 40 × 40 page filled with tiling pattern `P0`, whose matrix flips y so that its cell
/// (`/BBox [0 -40 40 0]`) covers the page; the cell paints shading `Sh0` with `sh`.
fn page_with_shading_in_a_flipped_cell(shading_dict: &str, shading_data: &[u8]) -> Vec<u8> {
    let content = "/Pattern cs /P0 scn 0 0 40 40 re f";
    let cell = "/Sh0 sh";
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    let mut push = |out: &mut Vec<u8>, head: String, data: &[u8]| {
        offsets.push(out.len());
        out.extend_from_slice(head.as_bytes());
        if head.ends_with("stream\n") {
            out.extend_from_slice(data);
            out.extend_from_slice(b"\nendstream");
        }
        out.extend_from_slice(b"\nendobj\n");
    };
    push(&mut out, "1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>".into(), &[]);
    push(&mut out, "2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(), &[]);
    push(
        &mut out,
        "3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 40 40] /Contents 4 0 R /Resources << /Pattern << /P0 5 0 R >> >> >>".into(),
        &[],
    );
    push(&mut out, format!("4 0 obj\n<< /Length {} >>\nstream\n", content.len()), content.as_bytes());
    push(
        &mut out,
        format!(
            "5 0 obj\n<< /Type /Pattern /PatternType 1 /PaintType 1 /TilingType 1 /BBox [0 -40 40 0] /XStep 40 /YStep 40 \
             /Matrix [1 0 0 -1 0 0] /Resources << /Shading << /Sh0 6 0 R >> >> /Length {} >>\nstream\n",
            cell.len()
        ),
        cell.as_bytes(),
    );
    if shading_data.is_empty() {
        push(&mut out, format!("6 0 obj\n<< {shading_dict} >>"), &[]);
    } else {
        push(&mut out, format!("6 0 obj\n<< {shading_dict} /Length {} >>\nstream\n", shading_data.len()), shading_data);
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len() + 1).as_bytes());
    for offset in &offsets {
        out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", offsets.len() + 1).as_bytes());
    out
}

/// The first page of `pdf` at one pixel a point over white.
fn render_first_page(pdf: Vec<u8>) -> Pixmap {
    let pdf = Pdf::new(pdf).expect("the fixture parses");
    let cache = RenderCache::new();
    let settings = RenderSettings {
        bg_color: WHITE,
        ..RenderSettings::default()
    };
    render(
        pdf.pages().get(0).expect("the fixture has one page"),
        &cache,
        &InterpreterSettings::default(),
        &settings,
    )
}

/// The RGBA pixel at (`x`, `y`) of a 40 × 40 render.
fn pixel(page: &Pixmap, x: u16, y: u16) -> [u8; 4] {
    let p = page.sample(x, y);
    [p.r, p.g, p.b, p.a]
}

/// A red free-form mesh (two triangles, 32-bit coordinates decoded onto the cell's `/BBox`, as
/// in PDFBOX-5852) and a red-to-blue axial shading, each painted with `sh` in a cell at negative
/// pattern y, cover the page.
#[test]
fn a_shading_in_a_cell_at_negative_pattern_coordinates_is_painted() {
    // Flag, x, y (32 bits each), r, g, b for each vertex: (0, -40), (40, -40), (0, 0), then
    // (40, -40), (40, 0), (0, 0).
    let vertex = |x: u32, y: u32| -> Vec<u8> { [vec![0u8], x.to_be_bytes().to_vec(), y.to_be_bytes().to_vec(), vec![255, 0, 0]].concat() };
    let (lo, hi) = (0u32, u32::MAX);
    let mesh = [vertex(lo, lo), vertex(hi, lo), vertex(lo, hi), vertex(hi, lo), vertex(hi, hi), vertex(lo, hi)].concat();
    let mesh_dict = "/ShadingType 4 /ColorSpace /DeviceRGB /BitsPerCoordinate 32 /BitsPerComponent 8 /BitsPerFlag 8 \
                     /Decode [0 40 -40 0 0 1 0 1 0 1]";
    let axial_dict = "/ShadingType 2 /ColorSpace /DeviceRGB /Coords [0 -40 40 -40] \
                      /Function << /FunctionType 2 /Domain [0 1] /C0 [1 0 0] /C1 [0 0 1] /N 1 >>";

    for (name, dict, data) in [("mesh", mesh_dict, mesh.as_slice()), ("axial", axial_dict, &[][..])] {
        let page = render_first_page(page_with_shading_in_a_flipped_cell(dict, data));
        assert_eq!((page.width(), page.height()), (40, 40), "{name}");
        for (x, y) in [(5, 5), (20, 20), (34, 34), (5, 34), (34, 5)] {
            let [r, g, b, a] = pixel(&page, x, y);
            assert!(a == 255 && g < 40 && r.saturating_add(b) > 200, "{name}: the cell's shading covers ({x}, {y}), got {:?}", [r, g, b, a]);
        }
    }
}
