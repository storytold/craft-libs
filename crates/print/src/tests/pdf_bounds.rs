use super::*;

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

#[test]
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

#[test]
fn nondefault_or_invalid_user_unit_is_rejected_before_any_update() {
    for unit in ["2", "0", "-1", "(invalid)", "5 0 R"] {
        for compressed in [false, true] {
            let pdf = fixture("3 0 R", compressed, &format!("/UserUnit {unit}"));
            assert!(matches!(read(&pdf), Err(Error::Spool(message)) if message.contains("UserUnit")));
        }
    }
    for unit in ["", "/UserUnit 1"] {
        assert!(read(&fixture("3 0 R", false, unit)).is_ok());
    }
    for (unit, accepted) in [("1      ", true), ("2      ", false)] {
        let pdf = fixture("3 0 R", false, "/UserUnit 5 0 R");
        let mut pdf = pdf;
        let at = pdf.windows(7).position(|bytes| bytes == b"[3 0 R]").unwrap();
        pdf[at..at + 7].copy_from_slice(unit.as_bytes());
        assert_eq!(read(&pdf).is_ok(), accepted, "indirect numeric UserUnit");
    }
}

fn xref_bomb(count: usize, index: Option<&str>) -> Vec<u8> {
    let packed = miniz_oxide::deflate::compress_to_vec_zlib(&vec![0u8; count], 6);
    let mut pdf = b"%PDF-1.7\n".to_vec();
    let start = pdf.len();
    let index = index.map(|value| format!("/Index {value}")).unwrap_or_default();
    pdf.extend_from_slice(
        format!("1 0 obj\n<< /Type /XRef /Size {count} {index} /W [0 1 0] /Filter /FlateDecode /Length {} >>\nstream\n", packed.len()).as_bytes(),
    );
    pdf.extend_from_slice(&packed);
    pdf.extend_from_slice(format!("\nendstream\nendobj\nstartxref\n{start}\n%%EOF\n").as_bytes());
    pdf
}

#[test]
fn compressed_xref_record_counts_fail_without_expanding_the_map() {
    for index in [None, Some("[0 8000000]"), Some("[0 250001 0 250000]")] {
        let pdf = xref_bomb(8_000_000, index);
        let mut entries = HashMap::new();
        let mut budget = Budget::default();
        assert!(xref_section(&pdf, startxref(&pdf).unwrap(), &mut entries, &mut budget).is_none());
        assert!(entries.is_empty());
        assert_eq!(budget.decoded, 0, "record limits precede decompression");
        assert!(budget.exhausted);
        assert!(read(&pdf).is_err());
    }
}

#[test]
fn classic_xref_declared_count_is_bounded_before_reading_records() {
    let pdf = b"xref\n0 8000000\n";
    let mut entries = HashMap::new();
    let mut budget = Budget::default();
    assert!(xref_section(pdf, 0, &mut entries, &mut budget).is_none());
    assert!(budget.exhausted);
    assert!(entries.is_empty());
}

#[test]
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

#[test]
fn predictor_rows_cannot_allocate_beyond_decoded_budget() {
    let parms = vec![
        (b"Predictor".to_vec(), Obj::Num(b"12".to_vec())),
        (b"Columns".to_vec(), Obj::Num(b"1048576".to_vec())),
        (b"Colors".to_vec(), Obj::Num(b"32".to_vec())),
        (b"BitsPerComponent".to_vec(), Obj::Num(b"16".to_vec())),
    ];
    assert!(unpredict(&[0], Some(&parms), MAX_STREAM).is_none());
}

#[test]
fn reframe_keeps_array_order_and_treats_null_as_an_empty_page() {
    for (contents, expected) in [("[3 0 R 3 0 R]", 4), ("null", 2)] {
        let original = fixture(contents, false, "");
        let sheets = read(&original).unwrap();
        let updated = sheets.update(&plan(&sheets.sheets, (612.0, 792.0), Fit::Shrink { margin: 0.0 }).unwrap()).unwrap();
        let sheets = read(&updated).unwrap();
        let contents = get(&sheets.sheets[0].dict, b"Contents").unwrap().array().unwrap();
        assert_eq!(contents.len(), expected);
        for item in &contents[1..expected - 1] {
            assert_eq!(*item, Obj::Ref(3, 0));
        }
    }
    for contents in ["[5 0 R [3 0 R]]", "123", "(text)"] {
        assert!(read(&fixture(contents, false, "")).is_err());
    }
}

#[test]
fn separate_cached_object_streams_cannot_exceed_the_job_budget() {
    let mut pdf = b"%PDF-1.7\n".to_vec();
    let values = [
        (1, "<< /Type /Catalog /Pages 2 0 R >>"),
        (2, "<< /Type /Pages /Kids [4 0 R] /Count 1 >>"),
        (4, "<< /Type /Page /MediaBox [0 0 612 792] >>"),
    ];
    let mut offsets = Vec::new();
    for (i, (num, value)) in values.iter().enumerate() {
        offsets.push(pdf.len());
        let mut raw = format!("{num} 0 {value}").into_bytes();
        raw.resize(12 << 20, b' ');
        let packed = miniz_oxide::deflate::compress_to_vec_zlib(&raw, 6);
        pdf.extend_from_slice(
            format!("{} 0 obj\n<< /Type /ObjStm /N 1 /First 4 /Filter /FlateDecode /Length {} >>\nstream\n", i + 6, packed.len()).as_bytes(),
        );
        pdf.extend_from_slice(&packed);
        pdf.extend_from_slice(b"\nendstream\nendobj\n");
    }
    let xref = pdf.len();
    let mut rows = Vec::new();
    for num in 0..10 {
        let (kind, value, index) = match num {
            1 => (2u8, 6u32, 0u16),
            2 => (2, 7, 0),
            4 => (2, 8, 0),
            6..=8 => (1, u32::try_from(offsets[num - 6]).unwrap(), 0),
            9 => (1, u32::try_from(xref).unwrap(), 0),
            _ => (0, 0, 65535),
        };
        rows.push(kind);
        rows.extend_from_slice(&value.to_be_bytes());
        rows.extend_from_slice(&index.to_be_bytes());
    }
    pdf.extend_from_slice(format!("9 0 obj\n<< /Type /XRef /Size 10 /Root 1 0 R /W [1 4 2] /Length {} >>\nstream\n", rows.len()).as_bytes());
    pdf.extend_from_slice(&rows);
    pdf.extend_from_slice(format!("\nendstream\nendobj\nstartxref\n{xref}\n%%EOF\n").as_bytes());
    assert!(read(&pdf).is_err(), "three small compressed streams must not cache 36 MiB");
}

#[test]
fn duplicate_dictionary_keys_keep_the_last_value_and_last_position() {
    let dict = Lexer::new(b"<< /A 1 /B 2 /A 3 /C 4 /B 5 >>", 0).object(0).unwrap();
    let dict = dict.dict().unwrap();
    assert_eq!(
        dict.iter().map(|(key, value)| (key.as_slice(), value.int().unwrap())).collect::<Vec<_>>(),
        [(b"A".as_slice(), 3), (b"C".as_slice(), 4), (b"B".as_slice(), 5)]
    );
    let escaped = Lexer::new(b"<< /A#42 1 /C 2 /AB 3 >>", 0).object(0).unwrap();
    assert_eq!(escaped.dict().unwrap(), &vec![(b"C".to_vec(), Obj::Num(b"2".to_vec())), (b"AB".to_vec(), Obj::Num(b"3".to_vec()))]);
}

#[test]
fn an_inflate_limit_stops_later_stream_and_object_work() {
    let packed = miniz_oxide::deflate::compress_to_vec_zlib(&[0; 32], 6);
    let dict = vec![(b"Filter".to_vec(), Obj::Name(b"FlateDecode".to_vec()))];
    let mut budget = Budget { decoded: MAX_DECODED - 4, ..Budget::default() };
    assert!(decode(&dict, &packed, &mut budget).is_none());
    assert!(budget.exhausted);
    // A rejected inflate has not charged its partial output. A later tiny stream
    // would otherwise fit, letting geometry fallback repeatedly try more streams.
    assert_eq!(budget.decoded, MAX_DECODED - 4);
    assert!(decode(&vec![], b"ok", &mut budget).is_none());
    assert!(budget.charge(1, 0, 0, 0).is_none());
    let mut reader = Reader { data: b"1 0 obj (hello) endobj", entries: HashMap::from([(1, Entry::At(0))]), objstms: HashMap::new(), budget };
    assert!(reader.object(1).is_none());
    assert_eq!(reader.budget.nodes, 0);
    assert_eq!(reader.budget.tokens, 0);
}

#[test]
fn an_oversized_inherited_media_box_is_not_copied_for_every_page() {
    // One inline MediaBox far longer than four numbers, inherited by many small pages.
    let pages = 4_000;
    let media = vec!["0"; 25_000].join(" ");
    let kids: Vec<String> = (0..pages).map(|i| format!("{} 0 R", i + 3)).collect();
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        format!("<< /Type /Pages /Kids [{}] /Count {pages} /MediaBox [{media}] >>", kids.join(" ")),
    ];
    objects.extend((0..pages).map(|_| "<< /Type /Page /Parent 2 0 R >>".to_string()));
    let mut pdf = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, object) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.extend_from_slice(format!("{} 0 obj\n{object}\nendobj\n", i + 1).as_bytes());
    }
    let xref = pdf.len();
    pdf.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f\r\n", objects.len() + 1).as_bytes());
    for offset in offsets {
        pdf.extend_from_slice(format!("{offset:010} 00000 n\r\n").as_bytes());
    }
    pdf.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1).as_bytes());
    let started = std::time::Instant::now();
    let sheets = read(&pdf).unwrap();
    assert!(started.elapsed() < std::time::Duration::from_secs(10), "{:?}", started.elapsed());
    assert_eq!(sheets.sheets.len(), pages);
}

/// A classic-xref PDF of these objects, numbered from 1.
fn classic(objects: &[String]) -> Vec<u8> {
    let mut pdf = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, object) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.extend_from_slice(format!("{} 0 obj\n{object}\nendobj\n", i + 1).as_bytes());
    }
    let xref = pdf.len();
    pdf.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f\r\n", objects.len() + 1).as_bytes());
    for offset in offsets {
        pdf.extend_from_slice(format!("{offset:010} 00000 n\r\n").as_bytes());
    }
    pdf.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1).as_bytes());
    pdf
}

/// Three Letter pages that inherit their MediaBox and Resources; the third is turned by its own
/// `/Rotate 90`, and carries `extra` in its dictionary.
fn three_pages(extra: &str) -> Vec<u8> {
    classic(&three_page_objects(extra))
}

fn three_page_objects(extra: &str) -> Vec<String> {
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R /PageMode /UseNone >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R 4 0 R 5 0 R] /Count 3 /MediaBox [0 0 612 792] /Resources << /ProcSet [/PDF] >> >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /Contents 6 0 R >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /Contents 7 0 R >>".to_string(),
        format!("<< /Type /Page /Parent 2 0 R /Contents 8 0 R /Rotate 90 {extra} >>"),
    ];
    objects.extend((0..3).map(|_| "<< /Length 0 >>\nstream\n\nendstream".to_string()));
    objects
}

/// `pdf` with `copies` collated copies in one job, or `None` when they can't be.
fn repeat(pdf: &[u8], copies: u32, two_sided: bool) -> Option<Vec<u8>> {
    read(pdf).unwrap().repeated(&[], Repeat { copies, two_sided }).unwrap()
}

#[test]
fn collated_copies_repeat_the_pages_with_a_blank_back_between_odd_copies() {
    let pdf = three_pages("");
    let sheets = read(&pdf).unwrap();
    // The turned page comes back upright in a portrait job, in every copy.
    let edits = plan(&sheets.sheets, (612.0, 792.0), Fit::Exact).unwrap();
    let content = |n: u32| Some(Obj::Ref(n, 0));
    for (two_sided, expected) in [
        (true, vec![content(6), content(7), content(8), None, content(6), content(7), content(8)]),
        (false, vec![content(6), content(7), content(8), content(6), content(7), content(8)]),
    ] {
        let updated = sheets.repeated(&edits, Repeat { copies: 2, two_sided }).unwrap().unwrap();
        let repeated = read(&updated).unwrap();
        let contents: Vec<Option<Obj>> = repeated.sheets.iter().map(|s| get(&s.dict, b"Contents").cloned()).collect();
        assert_eq!(contents, expected);
        for sheet in &repeated.sheets {
            assert_eq!((sheet.size(), sheet.rotate % 180), ((612.0, 792.0), 0), "{sheet:?}");
            // What the old parent gave each page is the page's own now.
            assert!(get(&sheet.dict, b"Resources").is_some());
            assert!(sheet.obj.is_some_and(|(num, _)| num >= sheets.size), "every copy is a new page object");
        }
        assert_eq!(get(&repeated.catalog, b"PageMode"), get(&sheets.catalog, b"PageMode"), "the rest of the catalog is kept");
        // The inherited resources are written once and shared.
        let resources: Vec<_> = repeated.sheets.iter().filter(|s| get(&s.dict, b"Contents").is_some()).map(|s| get(&s.dict, b"Resources")).collect();
        assert!(matches!(resources[0], Some(Obj::Ref(..))) && resources.iter().all(|r| *r == resources[0]), "{resources:?}");
    }
    // Without the edits the last page stays turned, and so does the blank back after it.
    let bytes = repeat(&pdf, 2, true).unwrap();
    let repeated = read(&bytes).unwrap();
    assert_eq!((repeated.sheets[3].size(), repeated.sheets[3].rotate), (repeated.sheets[2].size(), 90));
    // An even page count needs no blank; one copy is the document as it is.
    let even = classic(&[
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 /MediaBox [0 0 612 792] >>".into(),
        "<< /Type /Page /Parent 2 0 R >>".into(),
        "<< /Type /Page /Parent 2 0 R >>".into(),
    ]);
    assert_eq!(read(&repeat(&even, 3, true).unwrap()).unwrap().sheets.len(), 6);
    assert_eq!(repeat(&pdf, 1, true).unwrap(), pdf);
    // A cross-reference stream, with the catalog and the page in an object stream.
    for compressed in [false, true] {
        let bytes = repeat(&fixture("3 0 R", compressed, ""), 3, false).unwrap();
        let repeated = read(&bytes).unwrap();
        assert_eq!(repeated.sheets.len(), 3);
        assert_eq!(repeated.xref_stream, compressed);
    }
}

#[test]
fn collated_copies_that_cannot_be_one_job_are_left_to_separate_jobs() {
    assert!(repeat(&three_pages("/Annots [6 0 R]"), 2, false).is_none(), "annotations");
    let pdf = three_pages("");
    assert!(repeat(&pdf, 50_000, false).is_none(), "too many pages");
    let encrypted = Sheets { trailer: vec![(b"Encrypt".to_vec(), Obj::Ref(9, 0))], ..read(&pdf).unwrap() };
    assert!(encrypted.repeated(&[], Repeat { copies: 2, two_sided: false }).unwrap().is_none(), "encrypted");
}

#[test]
fn a_null_missing_or_free_entry_inherits_and_an_empty_annotation_list_is_none() {
    // Object 9 is written, but its cross-reference entry is free.
    let mut objects = three_page_objects("/Resources 9 0 R");
    objects.push("<< /ProcSet [/Text] >>".into());
    let mut free = classic(&objects);
    let at = free.windows(9).rposition(|w| w == b"00000 n\r\n").unwrap();
    free[at + 6] = b'f';
    for (extra, pdf) in [
        ("/Resources null", three_pages("/Resources null")),
        ("/Resources 99 0 R", three_pages("/Resources 99 0 R")),
        ("/Resources 9 0 R (free)", free),
        ("/Annots null", three_pages("/Annots null")),
        ("/Annots []", three_pages("/Annots []")),
    ] {
        let bytes = repeat(&pdf, 2, false).unwrap();
        let repeated = read(&bytes).unwrap();
        let resources: Vec<_> = repeated.sheets.iter().map(|s| get(&s.dict, b"Resources")).collect();
        assert!(matches!(resources[0], Some(Obj::Ref(..))) && resources.iter().all(|r| *r == resources[0]), "{extra}: {resources:?}");
    }
}

#[test]
fn large_inherited_resources_are_written_once_for_every_copy() {
    let pages = 4_000;
    let procset = vec!["/PDF"; 25_000].join(" ");
    let kids: Vec<String> = (0..pages).map(|i| format!("{} 0 R", i + 3)).collect();
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        format!("<< /Type /Pages /Kids [{}] /Count {pages} /MediaBox [0 0 612 792] /Resources << /ProcSet [{procset}] >> >>", kids.join(" ")),
    ];
    objects.extend((0..pages).map(|_| "<< /Type /Page /Parent 2 0 R >>".to_string()));
    let pdf = classic(&objects);
    let updated = repeat(&pdf, 3, false).unwrap();
    assert!(updated.len() < pdf.len() + (4 << 20), "{} bytes", updated.len());
    assert_eq!(read(&updated).unwrap().sheets.len(), 3 * pages);
}

fn flate_dict() -> Dict {
    vec![(b"Filter".to_vec(), Obj::Name(b"FlateDecode".to_vec()))]
}

fn varied(len: usize) -> Vec<u8> {
    (0..len).map(|i| u8::try_from(i * i % 251).unwrap_or(0)).collect()
}

#[test]
fn a_valid_flate_stream_round_trips() {
    let raw = varied(4000);
    let packed = miniz_oxide::deflate::compress_to_vec_zlib(&raw, 6);
    let mut budget = Budget::default();
    assert_eq!(decode(&flate_dict(), &packed, &mut budget), Some(raw.clone()));
    assert_eq!(budget.decoded, raw.len());
    assert!(!budget.exhausted);
}

#[test]
fn a_truncated_flate_stream_yields_only_the_bytes_it_inflated() {
    let raw = varied(4000);
    let packed = miniz_oxide::deflate::compress_to_vec_zlib(&raw, 6);
    let cut = packed.get(..packed.len() * 3 / 4).unwrap_or_default();
    let mut budget = Budget::default();
    let got = decode(&flate_dict(), cut, &mut budget).unwrap_or_default();
    assert!(!got.is_empty() && got.len() < raw.len());
    assert!(raw.starts_with(&got), "a prefix of the data, with no zero padding");
    assert!(!budget.exhausted);
}

#[test]
fn a_stream_rejected_before_any_output_is_none_not_zeros() {
    // A zlib header, then a reserved deflate block type: nothing is ever inflated, and the
    // bare-deflate retry rejects it too.
    let bad = [0x78, 0x9c, 0x07, 0xff, 0xff, 0xff, 0xff, 0xff];
    let mut budget = Budget::default();
    assert!(decode(&flate_dict(), &bad, &mut budget).is_none());
    assert_eq!(budget.decoded, 0);
    assert!(!budget.exhausted);
}

#[test]
fn a_bare_deflate_stream_still_decodes_through_the_retry() {
    let raw = varied(500);
    let packed = miniz_oxide::deflate::compress_to_vec(&raw, 6);
    let mut budget = Budget::default();
    assert_eq!(decode(&flate_dict(), &packed, &mut budget), Some(raw));
}

#[test]
fn an_over_limit_flate_stream_sets_exhausted() {
    let packed = miniz_oxide::deflate::compress_to_vec_zlib(&vec![7u8; 4096], 6);
    let mut budget = Budget { decoded: MAX_DECODED - 100, ..Budget::default() };
    assert!(decode(&flate_dict(), &packed, &mut budget).is_none());
    assert!(budget.exhausted);
    assert_eq!(budget.decoded, MAX_DECODED - 100);
}
