use super::*;

const EPS: &[u8] = b"%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 100 50\n%%HiResBoundingBox: 0 0 100.5 50.25\nnewpath\n%%EOF\n";

/// A binary header written by hand, as the specification lays it out.
fn dos(ps: &[u8], metafile: &[u8], tiff: &[u8]) -> Vec<u8> {
    let mut out = DOS_MAGIC.to_vec();
    let metafile_at = DOS_HEADER + ps.len();
    let at = |start: usize, part: &[u8]| if part.is_empty() { 0 } else { start };
    for v in [DOS_HEADER, ps.len(), at(metafile_at, metafile), metafile.len(), at(metafile_at + metafile.len(), tiff), tiff.len()] {
        out.extend((v as u32).to_le_bytes());
    }
    out.extend([0xFF, 0xFF]);
    for part in [ps, metafile, tiff] {
        out.extend_from_slice(part);
    }
    out
}

#[test]
fn sections_of_plain_and_binary_files() {
    assert_eq!(sections(EPS), Ok(Sections { postscript: EPS, tiff: None, metafile: None }));
    let file = dos(EPS, b"", b"II*\0tiff");
    assert_eq!(sections(&file), Ok(Sections { postscript: EPS, tiff: Some(b"II*\0tiff"), metafile: None }));
    // A metafile preview alone is not read as a TIFF preview.
    let file = dos(EPS, b"WMFDATA!", b"");
    assert_eq!(sections(&file), Ok(Sections { postscript: EPS, tiff: None, metafile: Some(b"WMFDATA!") }));
    let file = dos(EPS, b"WMF", b"TIFF");
    assert_eq!(sections(&file), Ok(Sections { postscript: EPS, tiff: Some(b"TIFF"), metafile: Some(b"WMF") }));
}

#[test]
fn a_header_that_points_outside_the_file_is_refused() {
    let file = dos(EPS, b"WMF", b"TIFF");
    // Cut anywhere, the header or a section it names is missing.
    for len in 4..file.len() {
        assert_eq!(sections(&file[..len]), Err(Error::OutsideFile), "cut at {len}");
    }
    // Offsets and lengths whose sum overflows 32 bits (and, on 32-bit targets, usize).
    for field in [4, 12, 20] {
        let mut bad = file.clone();
        bad[field..field + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        bad[field + 4..field + 8].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(sections(&bad), Err(Error::OutsideFile));
    }
    assert!(sniff(&file[..10]).is_some_and(|s| s.kind == Kind::Eps && !s.illustrator));
}

#[test]
fn wrap_writes_what_sections_reads() {
    assert_eq!(wrap(EPS, None, None).unwrap(), EPS, "no preview, no header");
    assert_eq!(wrap(EPS, Some(b""), Some(b"")).unwrap(), EPS);
    for (tiff, metafile) in [(Some(&b"TIFF"[..]), None), (None, Some(&b"WMF"[..])), (Some(&b"TIFF"[..]), Some(&b"WMF"[..]))] {
        let file = wrap(EPS, tiff, metafile).unwrap();
        assert_eq!(file, dos(EPS, metafile.unwrap_or_default(), tiff.unwrap_or_default()));
        assert_eq!(sections(&file), Ok(Sections { postscript: EPS, tiff, metafile }));
        assert_eq!(&file[28..30], [0xFF, 0xFF], "no checksum");
    }
}

fn boxes(ps: &str) -> Option<[f64; 4]> {
    Comments::read(ps.as_bytes()).page_box()
}

#[test]
fn bounding_boxes_of_the_header() {
    let c = Comments::read(EPS);
    assert_eq!((c.bounding_box, c.hires_bounding_box), (Some([0.0, 0.0, 100.0, 50.0]), Some([0.0, 0.0, 100.5, 50.25])));
    assert_eq!(c.page_box(), Some([0.0, 0.0, 100.5, 50.25]));
    let bb = |line: &str| boxes(&format!("%!PS-Adobe-3.0 EPSF-3.0\n{line}\n"));
    assert_eq!(bb("%%BoundingBox: 0 0 100 50"), Some([0.0, 0.0, 100.0, 50.0]));
    assert_eq!(bb("%%BoundingBox: 0 0 inf 50"), None);
    assert_eq!(bb("%%BoundingBox: -inf 0 100 50"), None);
    assert_eq!(bb("%%BoundingBox: -1e308 0 1e308 50"), None, "a size that overflows");
    assert_eq!(bb("%%BoundingBox: 100 50 0 0"), None);
    assert_eq!(bb("%%BoundingBox: 0 0 100"), None);
    assert_eq!(bb("%%BoundingBox: 0 0 100 50 7"), None);
    // A bad high-resolution box falls back to the plain one.
    assert_eq!(bb("%%HiResBoundingBox: 0 0 inf inf\n%%BoundingBox: 0 0 10 20"), Some([0.0, 0.0, 10.0, 20.0]));
    // Lines ending in a carriage return alone, and the first of a comment given twice.
    assert_eq!(boxes("%!PS-Adobe-3.0\r%%BoundingBox: 1 2 3 4\r%%BoundingBox: 5 6 7 8\r"), Some([1.0, 2.0, 3.0, 4.0]));
    // Comments past the header are the program's, not the file's.
    assert_eq!(boxes("%!PS-Adobe-3.0\n%%EndComments\n%%BoundingBox: 1 2 3 4\n"), None);
    assert_eq!(boxes("%!PS-Adobe-3.0\nnewpath\n%%BoundingBox: 1 2 3 4\n"), None);
    assert_eq!(boxes("%!PS-Adobe-3.0\n%%BeginProlog\n%%BoundingBox: 1 2 3 4\n"), None);
}

#[test]
fn a_deferred_bounding_box_is_the_outer_trailers() {
    let ps = "%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: (atend)\n%%EndComments\nnewpath\n%%Trailer\n%%BoundingBox: 1 2 30 40\n%%EOF\n";
    assert_eq!(boxes(ps), Some([1.0, 2.0, 30.0, 40.0]));
    // The trailer of an embedded document is not the file's.
    let nested = "%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: (atend)\n%%EndComments\n%%BeginDocument: inner.eps\n%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: (atend)\n%%Trailer\n%%BoundingBox: 0 0 5 5\n%%EOF\n%%EndDocument\n%%Trailer\n%%BoundingBox: 1 2 30 40\n%%EOF\n";
    assert_eq!(boxes(nested), Some([1.0, 2.0, 30.0, 40.0]));
    let only_inner = "%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: (atend)\n%%EndComments\n%%BeginDocument: inner.eps\n%%Trailer\n%%BoundingBox: 0 0 5 5\n%%EndDocument\n%%Trailer\n%%EOF\n";
    assert_eq!(boxes(only_inner), None);
    // A header with its own box never takes one from the trailer.
    let own = "%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 9 9\n%%EndComments\n%%Trailer\n%%BoundingBox: 1 2 30 40\n%%EOF\n";
    assert_eq!(boxes(own), Some([0.0, 0.0, 9.0, 9.0]));
    // Only the high-resolution box deferred.
    let hires = "%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 9 9\n%%HiResBoundingBox: (atend)\n%%EndComments\n%%Trailer\n%%HiResBoundingBox: 0.5 0.5 8.5 8.5\n%%EOF\n";
    assert_eq!(boxes(hires), Some([0.5, 0.5, 8.5, 8.5]));
}

#[test]
fn header_and_trailer_scans_are_bounded() {
    // A box past the header window is not looked for.
    let far = format!("%!PS-Adobe-3.0\n{}%%BoundingBox: 1 2 3 4\n", "%%DocumentFonts: Helvetica\n".repeat(MAX_HEADER / 27 + 1));
    assert_eq!(boxes(&far), None);
    let near = format!("%!PS-Adobe-3.0\n{}%%BoundingBox: 1 2 3 4\n", "%%DocumentFonts: Helvetica\n".repeat(100));
    assert_eq!(boxes(&near), Some([1.0, 2.0, 3.0, 4.0]));
    // A deferred box before the trailer window is not found; inside it, it is.
    let filler = "0 0 moveto\n".repeat(MAX_TRAILER / 11 + 1);
    let before = format!("%!PS-Adobe-3.0\n%%BoundingBox: (atend)\n%%EndComments\n%%BoundingBox: 1 2 3 4\n{filler}%%EOF\n");
    assert_eq!(boxes(&before), None);
    let inside = format!("%!PS-Adobe-3.0\n%%BoundingBox: (atend)\n%%EndComments\n{filler}%%Trailer\n%%BoundingBox: 1 2 3 4\n%%EOF\n");
    assert_eq!(boxes(&inside), Some([1.0, 2.0, 3.0, 4.0]));
}

#[test]
fn other_header_comments() {
    let c = Comments::read(b"%!PS-Adobe-3.0\n%%Creator: (Some App 1.0)\n%%Title: (A drawing)\n%%Pages: 3\n%%LanguageLevel: 2\n%%EndComments\n");
    assert_eq!((c.creator.as_deref(), c.title.as_deref(), c.pages, c.language_level), (Some("Some App 1.0"), Some("A drawing"), Some(3), Some(2)));
    assert!(!c.illustrator);
    assert_eq!(Comments::read(b"%!PS-Adobe-3.0\n%%Title: plain.eps\n").title.as_deref(), Some("plain.eps"));
    assert_eq!(Comments::read(b"%!PS-Adobe-3.0\n%%Pages: (atend)\n").pages, None);
}

#[test]
fn files_are_told_apart_by_content() {
    let kind = |b: &[u8]| sniff(b).map(|s| (s.kind, s.illustrator));
    assert_eq!(kind(EPS), Some((Kind::Eps, false)));
    assert!(is_eps(EPS) && is_eps(&dos(EPS, b"", b"TIFF")));
    assert_eq!(kind(b"%!PS-Adobe-3.0\nnot encapsulated"), Some((Kind::PostScript, false)));
    assert!(!is_eps(b"%!PS-Adobe-3.0\nnot encapsulated"));
    assert_eq!(kind(b"%!\nshowpage"), Some((Kind::PostScript, false)));
    assert_eq!(kind(b"<svg/>"), None);
    assert_eq!(kind(b""), None);
    // PostScript-based Illustrator files: by creator, or by the format's own comments.
    let by_creator = b"%!PS-Adobe-3.0 EPSF-3.0\n%%Creator: Adobe Illustrator(R) 8.0\n%%BoundingBox: 0 0 1 1\n";
    assert_eq!(kind(by_creator), Some((Kind::Eps, true)));
    assert_eq!(kind(b"%!PS-Adobe-3.0\n%%BoundingBox: 0 0 1 1\n%AI5_FileFormat 3\n%%EndComments\n"), Some((Kind::PostScript, true)));
    assert_eq!(kind(&dos(by_creator, b"", b"TIFF")), Some((Kind::Eps, true)));
    // A comment of the format past the header is not the header's.
    assert_eq!(kind(b"%!PS-Adobe-3.0\n%%EndComments\n%AI5_BeginLayer\n"), Some((Kind::PostScript, false)));
    // PDF-based: the private data keys near the start or the end.
    assert_eq!(kind(b"%PDF-1.5\n1 0 obj<</Type/Catalog>>endobj\n"), Some((Kind::Pdf, false)));
    assert_eq!(kind(b"%PDF-1.5\n1 0 obj<</Illustrator 2 0 R>>\n2 0 obj<</Private<</AIPrivateData1 3 0 R>>>>"), Some((Kind::Pdf, true)));
    assert_eq!(kind(b"%PDF-1.6\n<</AIPDFPrivateData1 3 0 R>>"), Some((Kind::Pdf, true)));
    let mut big = b"%PDF-1.5\n".to_vec();
    big.extend(std::iter::repeat_n(b' ', MAX_PDF_SCAN));
    big.extend_from_slice(b"<</AIPrivateData1 3 0 R>>");
    big.extend(std::iter::repeat_n(b' ', MAX_PDF_SCAN));
    assert_eq!(kind(&big), Some((Kind::Pdf, false)), "the key is outside both windows");
    big.truncate(big.len() - 100);
    assert_eq!(kind(&big), Some((Kind::Pdf, true)));
}

#[test]
fn epsi_previews() {
    // 4 by 2 pixels, one bit each: 1 is black. Rows are padded to bytes.
    let ps = b"%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 4 2\n%%BeginPreview: 4 2 1 2\n%A0\n%50\n%%EndPreview\n";
    let b = epsi(ps).unwrap();
    assert_eq!((b.width, b.height), (4, 2));
    assert_eq!(b.gray, [0, 255, 0, 255, 255, 0, 255, 0]);
    // Eight bits a pixel: 0 is white, 255 black; hex in either case, split over lines anywhere.
    let ps = b"%%BeginPreview: 3 1 8 2\n%00f\n%F80\n%%EndPreview\n";
    assert_eq!(epsi(ps).unwrap().gray, [255, 0, 127]);
    // Two bits a pixel.
    assert_eq!(epsi(b"%%BeginPreview: 4 1 2 1\n%1B\n%%EndPreview\n").unwrap().gray, [255, 170, 85, 0]);
    // Missing samples, a line that is no comment, a depth or a size outside the limits: none.
    assert_eq!(epsi(b"%%BeginPreview: 4 2 1 2\n%A0\n%%EndPreview\n"), None);
    assert_eq!(epsi(b"%%BeginPreview: 4 2 1 2\n%A0\nnewpath\n%50\n%%EndPreview\n"), None);
    assert_eq!(epsi(b"%%BeginPreview: 4 2 3 2\n%A0\n%50\n%%EndPreview\n"), None);
    assert_eq!(epsi(b"%%BeginPreview: 0 2 1 2\n%A0\n%50\n%%EndPreview\n"), None);
    assert_eq!(epsi(format!("%%BeginPreview: {} 1 1 1\n%A0\n%%EndPreview\n", MAX_EPSI + 1).as_bytes()), None);
    assert_eq!(epsi(format!("%%BeginPreview: 1 {} 1 1\n%A0\n%%EndPreview\n", MAX_EPSI + 1).as_bytes()), None);
    assert_eq!(epsi(b"%%BeginPreview: 99999999999999999999 1 1 1\n%A0\n"), None);
    assert_eq!(epsi(EPS), None);
    // The largest size claimed without its samples allocates nothing of that size and is none.
    assert_eq!(epsi(format!("%%BeginPreview: {MAX_EPSI} {MAX_EPSI} 8 1\n%00\n%%EndPreview\n").as_bytes()), None);
}

#[test]
fn ascii85_round_trips() {
    for len in 0..40usize {
        let data: Vec<u8> = (0..len).map(|i| (i * 37 + len * 11) as u8).collect();
        let text = ascii85::encode(&data);
        assert!(text.ends_with("~>\n"));
        assert_eq!(ascii85::decode(text.as_bytes(), len), Some(data.clone()), "{len} bytes");
        if len > 0 {
            assert_eq!(ascii85::decode(text.as_bytes(), len - 1), None, "one byte over the limit");
        }
    }
    // Four zero bytes are one `z`; the published example of the encoding.
    assert_eq!(ascii85::encode(&[0; 8]), "zz~>\n");
    assert_eq!(ascii85::encode(b"Man "), "9jqo^~>\n");
    assert_eq!(ascii85::decode(b"9jqo^~>", 4), Some(b"Man ".to_vec()));
    assert_eq!(ascii85::decode(b" 9jq\no^ ~>trailing", 4), Some(b"Man ".to_vec()), "whitespace is ignored, the end ends it");
    // Long data is broken into lines, none starting with `%`.
    let data: Vec<u8> = (0..4000u32).flat_map(|i| (i.wrapping_mul(2_654_435_761)).to_be_bytes()).collect();
    let text = ascii85::encode(&data);
    assert!(text.lines().all(|l| !l.starts_with('%') && l.len() <= 80), "{text}");
    assert!(ascii85::encode(&[0x0C, 0x80, 0, 0, 1, 2, 3, 4]).lines().all(|l| !l.starts_with('%')));
    assert_eq!(ascii85::decode(text.as_bytes(), data.len()), Some(data));
}

#[test]
fn ascii85_refuses_malformed_and_oversized_text() {
    assert_eq!(ascii85::decode(b"9jqo^", 100), None, "no end");
    assert_eq!(ascii85::decode(b"9jqo^~", 100), None);
    assert_eq!(ascii85::decode(b"9~>", 100), None, "one left-over character");
    assert_eq!(ascii85::decode(b"9jzo^~>", 100), None, "z inside a group");
    assert_eq!(ascii85::decode(b"9jqov~>", 100), None, "a character outside the alphabet");
    assert_eq!(ascii85::decode(b"s8W-\"~>", 100), None, "a group past 2^32");
    assert_eq!(ascii85::decode(b"s8W-!~>", 100), Some(vec![0xFF; 4]));
    // A megabyte of `z` is four megabytes: refused at the limit, not after allocating them all.
    let bomb = [vec![b'z'; 1 << 20], b"~>".to_vec()].concat();
    assert_eq!(ascii85::decode(&bomb, 1 << 16), None);
    assert_eq!(ascii85::decode(&bomb, 4 << 20).map(|v| v.len()), Some(4 << 20));
}
