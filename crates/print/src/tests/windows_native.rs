//! The Windows backend's tests: the printer list, the sheets read from a print-ready PDF and
//! the incremental update that gives them one shape (all platforms), and a job reaching a
//! paused test printer (Windows).

use craft_ui::testing::trial;

/// One page of a test PDF: its MediaBox (`None` = inherited from the page tree's root, which is
/// Letter), CropBox and `/Rotate`.
#[derive(Clone, Copy)]
struct TestPage {
    media: Option<[f64; 4]>,
    crop: Option<[f64; 4]>,
    rotate: Option<i32>,
}

const LETTER: TestPage = TestPage { media: Some([0.0, 0.0, 612.0, 792.0]), crop: None, rotate: None };
const TABLOID: TestPage = TestPage { media: Some([0.0, 0.0, 792.0, 1224.0]), crop: None, rotate: None };

/// A print-ready PDF of `pages`, each with a small black square: a classic cross-reference
/// table, or (`compressed`) the pages inside a Flate object stream with a predicted
/// cross-reference stream, as modern writers save. The root page node sets a Letter MediaBox
/// for pages that inherit it.
fn test_pdf(pages: &[TestPage], compressed: bool) -> Vec<u8> {
    test_pdf_with_annotations(pages, compressed, None)
}

fn test_pdf_with_annotations(pages: &[TestPage], compressed: bool, annotations: Option<&str>) -> Vec<u8> {
    let n = pages.len();
    // 1 catalog, 2 page tree, 3 content, then the pages.
    let kids: Vec<String> = (0..n).map(|i| format!("{} 0 R", 4 + i)).collect();
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        format!("<< /Type /Pages /Kids [{}] /Count {n} /MediaBox [0 0 612 792] >>", kids.join(" ")),
        "<< /Length 26 >>\nstream\n0 0 0 rg 36 36 72 72 re f\nendstream".to_string(),
    ];
    let rect = |r: [f64; 4]| format!("[{} {} {} {}]", r[0], r[1], r[2], r[3]);
    for p in pages {
        let mut page = "<< /Type /Page /Parent 2 0 R /Resources << >> /Contents 3 0 R".to_string();
        if let Some(m) = p.media {
            page += &format!(" /MediaBox {}", rect(m));
        }
        if let Some(c) = p.crop {
            page += &format!(" /CropBox {}", rect(c));
        }
        if let Some(r) = p.rotate {
            page += &format!(" /Rotate {r}");
        }
        if let Some(annotations) = annotations {
            page += &format!(" /Annots {annotations}");
        }
        objects.push(page + " >>");
    }
    let mut pdf = b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n".to_vec();
    if !compressed {
        let mut offsets = Vec::new();
        for (i, object) in objects.iter().enumerate() {
            offsets.push(pdf.len());
            pdf.extend_from_slice(format!("{} 0 obj\n{object}\nendobj\n", i + 1).as_bytes());
        }
        let xref = pdf.len();
        pdf.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f\r\n", offsets.len() + 1).as_bytes());
        for offset in &offsets {
            pdf.extend_from_slice(format!("{offset:010} 00000 n\r\n").as_bytes());
        }
        pdf.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", offsets.len() + 1).as_bytes());
        return pdf;
    }
    // Objects 1, 2 and the pages go in object stream `stm`; the content stream can't.
    let stm = objects.len() + 1;
    let inside: Vec<usize> = (1..=objects.len()).filter(|&i| i != 3).collect();
    let (mut header, mut body) = (String::new(), String::new());
    for &i in &inside {
        header += &format!("{i} {} ", body.len());
        body += &objects[i - 1];
        body += "\n";
    }
    let packed = miniz_oxide::deflate::compress_to_vec_zlib(format!("{header}{body}").as_bytes(), 6);
    let content_at = pdf.len();
    pdf.extend_from_slice(format!("3 0 obj\n{}\nendobj\n", objects[2]).as_bytes());
    let stm_at = pdf.len();
    pdf.extend_from_slice(
        format!(
            "{stm} 0 obj\n<< /Type /ObjStm /N {} /First {} /Filter /FlateDecode /Length {} >>\nstream\n",
            inside.len(),
            header.len(),
            packed.len()
        )
        .as_bytes(),
    );
    pdf.extend_from_slice(&packed);
    pdf.extend_from_slice(b"\nendstream\nendobj\n");
    let xref_num = stm + 1;
    let xref_at = pdf.len();
    // Rows of 1 + 2 + 1 bytes, PNG "Up"-predicted (filter byte 2), as Acrobat writes them.
    let mut rows: Vec<[u8; 4]> = vec![[0, 0, 0, 255]];
    for i in 1..=xref_num {
        rows.push(match i {
            3 => [1, (content_at >> 8) as u8, content_at as u8, 0],
            i if i == stm => [1, (stm_at >> 8) as u8, stm_at as u8, 0],
            i if i == xref_num => [1, (xref_at >> 8) as u8, xref_at as u8, 0],
            i => [2, 0, stm as u8, inside.iter().position(|&x| x == i).unwrap() as u8],
        });
    }
    let mut predicted = Vec::new();
    let mut prev = [0u8; 4];
    for row in rows {
        predicted.push(2);
        predicted.extend(row.iter().zip(prev).map(|(b, p)| b.wrapping_sub(p)));
        prev = row;
    }
    let packed = miniz_oxide::deflate::compress_to_vec_zlib(&predicted, 6);
    pdf.extend_from_slice(
        format!(
            "{xref_num} 0 obj\n<< /Type /XRef /Size {} /W [1 2 1] /Root 1 0 R /Filter /FlateDecode /DecodeParms << /Predictor 12 /Columns 4 >> /Length {} >>\nstream\n",
            xref_num + 1,
            packed.len()
        )
        .as_bytes(),
    );
    pdf.extend_from_slice(&packed);
    pdf.extend_from_slice(format!("\nendstream\nendobj\nstartxref\n{xref_at}\n%%EOF\n").as_bytes());
    pdf
}

/// Each sheet's printed size (rounded to a point) and rotation.
fn shapes(pdf: &[u8]) -> Vec<((i64, i64), u32)> {
    let read = crate::sheets::read(pdf).unwrap();
    read.sheets.iter().map(|s| ((s.size().0.round() as i64, s.size().1.round() as i64), s.rotate)).collect()
}

/// The `q … cm … re W n` line a reframed page's content starts with.
fn frame_of(updated: &[u8], original: usize) -> String {
    let tail = String::from_utf8_lossy(&updated[original..]).into_owned();
    tail.lines().find(|l| l.starts_with("q ")).unwrap_or_default().to_string()
}

#[trial]
fn shrink_to_fit_reframes_sheets_on_the_paper() {
    use crate::sheets::{Fit, plan, read};
    // Tabloid onto Letter with an 18 pt margin: 756 / 1224 of the height wins, centred.
    let pdf = test_pdf(&[TABLOID], true);
    let sheets = read(&pdf).unwrap();
    let edits = plan(&sheets.sheets, (612.0, 792.0), Fit::Shrink { margin: 18.0 }).unwrap();
    let updated = sheets.update(&edits).unwrap();
    assert_eq!(shapes(&updated), [((612, 792), 0)]);
    assert_eq!(frame_of(&updated, pdf.len()), "q 0.6176 0 0 0.6176 61.4118 18 cm 0 0 792 1224 re W n");
    let tail = String::from_utf8_lossy(&updated[pdf.len()..]).into_owned();
    assert!(tail.contains("/Contents [7 0 R 3 0 R 8 0 R]"), "the page's own content between q … Q: {tail}");

    // A smaller sheet keeps its size, centred; a sheet with a CropBox is clipped to it, and the
    // crop is what lands centred.
    let card = TestPage { media: Some([0.0, 0.0, 360.0, 504.0]), ..LETTER };
    let cropped = TestPage { crop: Some([100.0, 100.0, 400.0, 500.0]), ..LETTER };
    let pdf = test_pdf(&[card, cropped], false);
    let sheets = read(&pdf).unwrap();
    let edits = plan(&sheets.sheets, (612.0, 792.0), Fit::Shrink { margin: 18.0 }).unwrap();
    assert_eq!(edits.len(), 2);
    assert_eq!(edits[0].frame.as_ref().unwrap().matrix, [1.0, 0.0, 0.0, 1.0, 126.0, 144.0], "never enlarged");
    assert_eq!(edits[1].frame.as_ref().unwrap().matrix, [1.0, 0.0, 0.0, 1.0, 56.0, 96.0]);
    assert_eq!(edits[1].frame.as_ref().unwrap().clip, [100.0, 100.0, 300.0, 400.0]);
    assert_eq!(shapes(&sheets.update(&edits).unwrap()), [((612, 792), 0), ((612, 792), 0)]);

    // The paper's own size: shrunk into the printable area when the printer has margins, as
    // CUPS' auto-fit does, and left alone when it prints to the edge.
    let pdf = test_pdf(&[LETTER], false);
    let sheets = read(&pdf).unwrap();
    assert_eq!(plan(&sheets.sheets, (612.0, 792.0), Fit::Shrink { margin: 12.0 }).unwrap().len(), 1);
    assert!(plan(&sheets.sheets, (612.0, 792.0), Fit::Shrink { margin: 0.0 }).unwrap().is_empty());

    // Sheets of the other orientation turn first, then fit. A Tabloid page with /Rotate 90 is a
    // landscape sheet: turned back to 0 and shrunk; a landscape Tabloid page is turned to 270.
    let pdf = test_pdf(&[LETTER, TestPage { media: Some([0.0, 0.0, 1224.0, 792.0]), ..LETTER }, TestPage { rotate: Some(90), ..TABLOID }], true);
    let sheets = read(&pdf).unwrap();
    let edits = plan(&sheets.sheets, (612.0, 792.0), Fit::Shrink { margin: 0.0 }).unwrap();
    assert_eq!(edits.iter().map(|e| (e.sheet, e.rotate)).collect::<Vec<_>>(), [(1, 270), (2, 0)]);
    assert_eq!(edits[0].frame.as_ref().unwrap().media, (792.0, 612.0), "the paper in the page's unrotated space");
    assert_eq!(shapes(&sheets.update(&edits).unwrap()), [((612, 792), 0), ((612, 792), 270), ((612, 792), 0)]);
}
