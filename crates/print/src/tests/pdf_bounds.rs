use super::*;
use craft_ui::testing::trial;

fn fixture(contents: &str, compressed: bool, unit: &str) -> Vec<u8> {
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [4 0 R] /Count 1 >>".to_string(),
        "<< /Length 26 >>\nstream\n0 0 0 rg 36 36 72 72 re f\nendstream".to_string(),
        format!("<< /Type /Page /Parent 2 0 R /Resources <<>> /MediaBox [0 0 792 1224] /Contents {contents} {unit} >>"),
        "[3 0 R]".to_string(),
    ];
    let mut pdf = b"%PDF-1.7\n".to_vec();
    if !compressed {
        let mut offsets = vec![0];
        for (i, object) in objects.iter().enumerate() {
            offsets.push(pdf.len());
            pdf.extend_from_slice(format!("{} 0 obj\n{object}\nendobj\n", i + 1).as_bytes());
        }
        let xref = pdf.len();
        pdf.extend_from_slice(b"xref\n0 6\n0000000000 65535 f\r\n");
        for offset in offsets.iter().skip(1) {
            pdf.extend_from_slice(format!("{offset:010} 00000 n\r\n").as_bytes());
        }
        pdf.extend_from_slice(format!("trailer\n<< /Size 6 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes());
        return pdf;
    }
    let content_at = pdf.len();
    pdf.extend_from_slice(format!("3 0 obj\n{}\nendobj\n", objects[2]).as_bytes());
    let (mut header, mut body) = (String::new(), String::new());
    let inside = [1usize, 2, 4, 5];
    for &i in &inside {
        header += &format!("{i} {} ", body.len());
        body += &objects[i - 1];
        body.push('\n');
    }
    let packed = miniz_oxide::deflate::compress_to_vec_zlib(format!("{header}{body}").as_bytes(), 6);
    let stm_at = pdf.len();
    pdf.extend_from_slice(
        format!("6 0 obj\n<< /Type /ObjStm /N 4 /First {} /Filter /FlateDecode /Length {} >>\nstream\n", header.len(), packed.len()).as_bytes(),
    );
    pdf.extend_from_slice(&packed);
    pdf.extend_from_slice(b"\nendstream\nendobj\n");
    let xref_at = pdf.len();
    let mut rows = Vec::new();
    for i in 0..8 {
        let (kind, value, generation) = match i {
            0 => (0u8, 0u32, 65535u16),
            3 => (1, u32::try_from(content_at).unwrap(), 0),
            6 => (1, u32::try_from(stm_at).unwrap(), 0),
            7 => (1, u32::try_from(xref_at).unwrap(), 0),
            i => (2, 6, u16::try_from(inside.iter().position(|&n| n == i).unwrap()).unwrap()),
        };
        rows.push(kind);
        rows.extend_from_slice(&value.to_be_bytes());
        rows.extend_from_slice(&generation.to_be_bytes());
    }
    pdf.extend_from_slice(format!("7 0 obj\n<< /Type /XRef /Size 8 /Root 1 0 R /W [1 4 2] /Length {} >>\nstream\n", rows.len()).as_bytes());
    pdf.extend_from_slice(&rows);
    pdf.extend_from_slice(format!("\nendstream\nendobj\nstartxref\n{xref_at}\n%%EOF\n").as_bytes());
    pdf
}

#[trial]
fn reframe_preserves_single_direct_and_indirect_content_streams() {
    for compressed in [false, true] {
        for (name, contents) in [("single", "3 0 R"), ("direct-array", "[3 0 R]"), ("indirect-array", "5 0 R")] {
            let original = fixture(contents, compressed, "");
            let sheets = read(&original).unwrap();
            let updated = sheets.update(&plan(&sheets.sheets, (612.0, 792.0), Fit::Shrink { margin: 18.0 }).unwrap()).unwrap();
            let sheets = read(&updated).unwrap();
            let contents = get(&sheets.sheets[0].dict, b"Contents").unwrap().array().unwrap();
            assert_eq!(contents.len(), 3);
            assert_eq!(contents[1], Obj::Ref(3, 0), "the square stream must survive, not a reference to the array");
            if let Some(dir) = std::env::var_os("CRAFT_PRINT_PDF_FIXTURES") {
                let dir = std::path::PathBuf::from(dir);
                std::fs::write(dir.join(format!("{name}-{compressed}-original.pdf")), &original).unwrap();
                std::fs::write(dir.join(format!("{name}-{compressed}-updated.pdf")), &updated).unwrap();
            }
        }
    }
}

#[trial]
fn object_stream_and_parsed_container_budgets_are_job_wide() {
    let dict = vec![];
    let mut budget = Budget { decoded: MAX_DECODED - 4, ..Budget::default() };
    assert_eq!(decode(&dict, b"1234", &mut budget), Some(b"1234".to_vec()));
    assert!(decode(&dict, b"1", &mut budget).is_none());
    assert!(budget.exhausted);

    let array = format!("[{}]", "null ".repeat(MAX_OBJECT_NODES));
    assert!(Lexer::new(array.as_bytes(), 0).object(0).is_none());
    let token = format!("({})", "x".repeat(MAX_OBJECT_TOKENS));
    assert!(Lexer::new(token.as_bytes(), 0).object(0).is_none());
    let mut budget = Budget { nodes: MAX_NODES, ..Budget::default() };
    let mut lx = Lexer::new(b"null", 0);
    assert!(lx.object(0).is_some());
    assert!(budget.parsed(&lx).is_none());
    assert!(budget.exhausted);
}
